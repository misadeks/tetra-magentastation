use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};
use std::panic;
pub use tetra_ms_core::mac::wire::FragEndKind;
use tetra_ms_core::mac::{self, Channel, FragmentAction, ReservedAction};
#[cfg(test)]
use tetra_ms_core::mac::{Allocation, AllocationType, Grant, GrantCapacity};
use tetra_ms_core::types::{BitError, BitStorage};

use tetra_config::magentastation::{SharedConfig, StackMode};
use tetra_core::freqs::FreqInfo;
use tetra_core::tetra_entities::TetraEntity;
use tetra_core::{BitBuffer, PhyBlockNum, PhysicalChannel, Sap, TdmaTime, TetraAddress, Todo, TxReporter, unimplemented_log};
use tetra_saps::tlmb::{TlmbSyncInd, TlmbSysinfoInd};
use tetra_saps::tma::TmaUnitdataInd;
use tetra_saps::tmv::TmvConfigureReq;
use tetra_saps::tmv::TmvTuneReq;
use tetra_saps::tmv::TmvTxTuneReq;
use tetra_saps::tmv::enums::logical_chans::LogicalChannel;
use tetra_saps::tmv::{TmvUnitdataReq, TmvUnitdataReqSlot, TxBurstPlacement};
use tetra_saps::{SapMsg, SapMsgInner};

#[cfg(test)]
use tetra_pdus::umac::enums::basic_slotgrant_cap_alloc::BasicSlotgrantCapAlloc;
#[cfg(test)]
use tetra_pdus::umac::enums::basic_slotgrant_granting_delay::BasicSlotgrantGrantingDelay;
#[cfg(test)]
use tetra_pdus::umac::fields::basic_slotgrant::BasicSlotgrant;
#[cfg(test)]
use tetra_pdus::umac::fields::channel_allocation::ChanAllocElement;
#[cfg(test)]
use tetra_pdus::umac::pdus::access_assign::AccessAssign;
use tetra_pdus::umac::pdus::access_define::AccessDefine;
#[cfg(test)]
use tetra_saps::lcmc::enums::alloc_type::ChanAllocType;
#[cfg(test)]
use tetra_saps::lcmc::enums::ul_dl_assignment::UlDlAssignment;

use crate::umac::subcomp::ms_defrag::MsDefrag;
#[cfg(test)]
use crate::umac::subcomp::ms_random_access::interpret_access_assign;
use crate::umac::subcomp::ms_random_access::{AccessCode, AccessParamStore, MsRandomAccess, RaAction, Subslot, ThreadRaRng};
use crate::{MessagePrio, MessageQueue, TetraEntityTrait};

#[cfg(test)]
const SCH_HU_TYPE1_BITS: usize = mac::wire::SCH_HU_BITS as usize;
#[cfg(test)]
const SCH_F_TYPE1_BITS: usize = mac::wire::SCH_F_BITS as usize;
#[cfg(test)]
const MAX_UL_FRAG_SLOTS: usize = mac::wire::MAX_UL_FRAG_SLOTS;
#[cfg(test)]
const BROADCAST_SSI: u32 = 0xFF_FFFF;
#[cfg(test)]
const UPLINK_AUDIO_MAX_FRAMES: usize = mac::UPLINK_AUDIO_MAX_FRAMES;

/// A MAC block built and queued for uplink transmission, awaiting a valid
/// access opportunity. ETSI TS 100 392-2 cl. 23.5 (MAC access). The random
/// access slot-selection algorithm (cl. 23.5.1.4) that decides *when* to send
/// this, and the actual transmission, are driven by the real-time uplink PHY
/// (Phase 3d).
#[derive(Debug, Clone)]
pub struct PendingUplink {
    /// The full type-1 MAC block (e.g. 92-bit SCH/HU), ready for LMAC encode.
    pub mac_block: BitBuffer,
    pub logical_channel: LogicalChannel,
    pub scrambling_code: u32,
    /// Transmit receipt shared with the LLC acknowledged-mode outbound entry
    /// (ETSI TS 100 392-2 cl. 22.3.2.3). The MS-MAC marks it *transmitted* once
    /// the BS acknowledges the random access (MAC-RESOURCE, cl. 23.5.1.4.8) and
    /// *discarded* if the access procedure is abandoned (cl. 23.5.1.4.9). This
    /// drives the LLC T251/N252 retransmit-and-give-up machinery; without it an
    /// acknowledged-mode uplink whose LLC N(R) ack is withheld by the BS would
    /// wedge the basic link forever. `None` for the frag-start of a fragmented
    /// transfer (the receipt travels with the completing MAC-END-HU instead).
    pub tx_reporter: Option<TxReporter>,
    /// L3-specified PDU priority for the random-access gate (ETSI TS 100 392-2
    /// cl. 23.5.1.4.4): the access attempt is only permitted when this priority
    /// is at least the access code's advertised minimum. `None` → the MAC uses
    /// the access-code minimum (unspecified/LLC-internal traffic always passes,
    /// preserving prior behaviour). Set from the L3 `TMA-UNITDATA-REQ`.
    pub pdu_priority: Option<u8>,
    /// L3 emergency flag (cl. 23.5.1.4.4): an emergency transfer on access code A
    /// bypasses the priority gate and doubles the maximum transmission count.
    /// Always `false` for MM signalling (emergency is CMCE-only); plumbed for a
    /// future CMCE MS.
    pub is_emergency: bool,
}

#[cfg(test)]
type CorePendingUplink = mac::PendingUplink<u32>;

/// The remaining TM-SDU of an uplink transfer that did not fit a single
/// MAC-ACCESS random-access burst and was fragmented (ETSI TS 100 392-2
/// cl. 23.4.2.1.2). The first fragment (a MAC-ACCESS "start of fragmentation"
/// carrying a capacity request) is transmitted by random access; this holds
/// the remainder, sent as a MAC-END-HU (subslot) or MAC-END-UL (full slot)
/// once the BS grants the requested uplink capacity (cl. 23.5.2 basic slot
/// granting).
pub type UplinkFragment = mac::UplinkFragment<u32>;

#[cfg(test)]
type ReservedTxPlan = mac::ReservedTxPlan<u32>;

/// A complete uplink transfer waiting behind the one currently in flight.
///
/// The MS-MAC can only have a single uplink transfer contending for random
/// access at a time (`pending_uplink`/`pending_fragment`, cl. 23.5.1.4). The
/// LLC, however, may hand down several TM-SDUs in quick succession — e.g. an
/// unacknowledged BL-ACK auto-ack interleaved with an acknowledged group-attach
/// — so extra transfers are held here in FIFO order and promoted into the
/// active slots by [`mac::MsMac::promote`] once the current one
/// completes. This prevents a later uplink from overwriting an in-flight
/// transfer and orphaning its [`PendingUplink::tx_reporter`], which would leave
/// the acknowledged basic link wedged (no `t_umac_done`, so the LLC T251/N252
/// retransmit-and-give-up never runs).
type QueuedUplink = mac::QueuedUplink<u32>;

/// Upper bound on [`UmacMs::uplink_queue`]. The basic link is effectively
/// stop-and-wait, so only a handful of transfers ever queue (a data PDU plus a
/// few auto-acks). The cap is a defensive guard against pathological growth; on
/// overflow the oldest queued transfer is dropped with its receipt marked
/// discarded so the LLC retransmits it rather than leaking memory.
#[cfg(test)]
const MAX_UPLINK_QUEUE: usize = mac::MAX_UPLINK_QUEUE;
const MAX_TX_REPORTERS: usize = mac::MAX_UPLINK_QUEUE + mac::MAX_STOLEN_SIGNALLING + 2;

pub struct UmacMs {
    mac: mac::MsMac,
    tx_reporters: BTreeMap<u32, TxReporter>,
    next_tx_reporter: u32,
    configured_role: StackMode,
    configured_issi: Option<u32>,
    self_component: TetraEntity,
    config: SharedConfig,
    defrag: MsDefrag,
    /// Random access parameters advertised by the serving cell, per access code
    /// (ACCESS-DEFINE cl. 21.4.4.3 + SYSINFO default-A cl. 21.4.4.1).
    access_params: AccessParamStore,

    /// MS-MAC random access state machine (cl. 23.5.1.4). Decides *when* the
    /// queued `pending_uplink` block may be transmitted on an access opportunity.
    random_access: MsRandomAccess,

    /// Last uplink carrier (Hz) derived from the serving cell's SYSINFO duplex
    /// parameters (band + main carrier + duplex spacing resolved through the
    /// programmed duplex table, EN 300 392-2 cl. 18.4.2.2 / cl. 21.4.4). The MS
    /// derives the uplink at camp — not from config — and requests the PHY
    /// retune the transmitter (`TmvTxTuneReq`) only when this value changes, so
    /// the retune is issued once per (re)selected cell rather than on every
    /// SYSINFO broadcast.
    derived_ul_freq: Option<u32>,
}

fn pack<const N: usize>(buffer: &BitBuffer) -> Result<BitStorage<N>, BitError> {
    if buffer.get_len() > N * 8 {
        return Err(BitError::Capacity);
    }
    let mut bits = BitStorage::new();
    for offset in 0..buffer.get_len() {
        bits.push(1, buffer.peek_bits_startoffset(offset, 1).ok_or(BitError::Truncated)? as u32)?;
    }
    Ok(bits)
}

fn unpack<const N: usize>(bits: &BitStorage<N>) -> BitBuffer {
    let mut buffer = BitBuffer::from_vec(bits.as_bytes().to_vec());
    buffer.set_raw_end(usize::from(bits.len()));
    buffer
}

fn desktop_channel(channel: Channel) -> LogicalChannel {
    match channel {
        Channel::SchHu => LogicalChannel::SchHu,
        Channel::SchF => LogicalChannel::SchF,
        Channel::Stch => LogicalChannel::Stch,
        Channel::TchS => LogicalChannel::TchS,
    }
}

impl Deref for UmacMs {
    type Target = mac::MsMac;

    fn deref(&self) -> &Self::Target {
        &self.mac
    }
}

impl DerefMut for UmacMs {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.mac
    }
}

impl Drop for UmacMs {
    fn drop(&mut self) {
        self.discard_reporters();
    }
}

impl UmacMs {
    pub fn new(config: SharedConfig) -> Self {
        Self::try_new(config).expect("MS attached groups exceed portable MAC capacity")
    }

    pub fn try_new(config: SharedConfig) -> Result<Self, BitError> {
        // Seed the runtime downlink address-filter set from config (cl.
        // 23.4.1.2.1). This preserves pre-registration behaviour: before any
        // MLE-IDENTITIES update the MS accepts its own ISSI and the configured
        // groups. Runtime group attach/detach later replaces the group set.
        let mut mac = mac::MsMac::default();
        let (configured_role, configured_issi) = {
            let cfg = config.config();
            if let Some(ms) = cfg.ms.as_ref() {
                mac.valid_individual_ssi = Some(ms.issi);
                mac.valid_group_ssis.replace(&ms.attach_groups)?;
            }
            (cfg.stack_mode, cfg.ms.as_ref().map(|ms| ms.issi))
        };
        Ok(Self {
            mac,
            tx_reporters: BTreeMap::new(),
            next_tx_reporter: 0,
            configured_role,
            configured_issi,
            self_component: TetraEntity::Umac,
            config,
            defrag: MsDefrag::new(),

            access_params: AccessParamStore::new(),
            random_access: MsRandomAccess::new(),
            derived_ul_freq: None,
        })
    }

    fn track_reporter(&mut self, reporter: Option<TxReporter>) -> Result<Option<u32>, TxReporter> {
        let Some(reporter) = reporter else { return Ok(None) };
        let Some(handle) = self.next_tx_reporter.checked_add(1) else {
            return Err(reporter);
        };
        if self.tx_reporters.len() >= MAX_TX_REPORTERS {
            return Err(reporter);
        }
        self.next_tx_reporter = handle;
        self.tx_reporters.insert(handle, reporter);
        Ok(Some(handle))
    }

    fn finish_receipt(&mut self, handle: Option<u32>, transmitted: bool) {
        let Some(handle) = handle else { return };
        match self.tx_reporters.remove(&handle) {
            Some(reporter) => {
                if transmitted {
                    reporter.try_mark_transmitted();
                } else {
                    reporter.try_mark_discarded();
                }
            }
            None => tracing::warn!(handle, "UMAC receipt has no desktop reporter"),
        }
    }

    fn discard_reporters(&mut self) {
        for (_, reporter) in std::mem::take(&mut self.tx_reporters) {
            reporter.try_mark_discarded();
        }
    }

    /// Cancel pending platform receipts and reset the numeric protocol state.
    /// Handle generations are deliberately not reused after reset.
    pub fn reset(&mut self) -> Result<(), BitError> {
        let mut groups = mac::IdentitySet::default();
        let (role, issi) = {
            let config = self.config.config();
            if let Some(ms) = &config.ms {
                groups.replace(&ms.attach_groups)?;
            }
            (config.stack_mode, config.ms.as_ref().map(|ms| ms.issi))
        };
        self.clear_protocol();
        self.mac.valid_group_ssis = groups;
        self.mac.valid_individual_ssi = issi;
        self.configured_role = role;
        self.configured_issi = issi;
        Ok(())
    }

    fn clear_protocol(&mut self) {
        self.discard_reporters();
        self.mac.reset();
        self.random_access.cancel();
        self.access_params = AccessParamStore::new();
        self.defrag.reset();
        self.derived_ul_freq = None;
    }

    fn synchronize_role(&mut self) {
        let changed = {
            let config = self.config.config();
            config.stack_mode != self.configured_role || config.ms.as_ref().map(|ms| ms.issi) != self.configured_issi
        };
        if changed {
            self.set_config(self.config.clone());
        }
    }

    fn rx_tmv_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        tracing::trace!("rx_tmv_prim");
        match message.msg {
            SapMsgInner::TmvUnitdataInd(_) => {
                self.rx_tmv_unitdata_ind(queue, message);
            }
            _ => {
                panic!();
            }
        }
    }

    pub fn rx_tmv_unitdata_ind(&mut self, queue: &mut MessageQueue, mut message: SapMsg) {
        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };
        tracing::trace!("rx_tmv_unitdata_ind: {:?}", prim.logical_channel);

        match prim.logical_channel {
            LogicalChannel::Aach => {
                self.rx_tmv_aach(queue, message);
            }

            LogicalChannel::Bsch => {
                self.rx_tmv_bsch(queue, message);
            }

            LogicalChannel::SchF => {
                // Full slot signalling
                assert!(
                    prim.block_num == PhyBlockNum::Both,
                    "{:?} can't have block_num {:?}",
                    prim.logical_channel,
                    prim.block_num
                );
                self.rx_tmv_sch(queue, message);
            }

            LogicalChannel::Bnch | LogicalChannel::Stch | LogicalChannel::SchHd => {
                // Half slot signalling
                assert!(
                    matches!(prim.block_num, PhyBlockNum::Block1 | PhyBlockNum::Block2),
                    "{:?} can't have block_num {:?}",
                    prim.logical_channel,
                    prim.block_num
                );
                self.rx_tmv_sch(queue, message);
            }
            _ => unreachable!("invalid channel: {:?}", prim.logical_channel),
        }
    }

    /// Receive signalling (SCH, or STCH / BNCH)
    pub fn rx_tmv_sch(&mut self, queue: &mut MessageQueue, mut message: SapMsg) {
        tracing::trace!("rx_tmv_sch");
        let SapMsgInner::TmvUnitdataInd(prim) = &message.msg else {
            panic!()
        };
        let mut cursor = mac::downlink::BlockCursor::new(prim.pdu.get_len().try_into().expect("MAC block length fits u16"));

        // Iterate until no more messages left in mac block
        loop {
            // Extract info from inner block
            let SapMsgInner::TmvUnitdataInd(prim) = &message.msg else {
                panic!()
            };
            let Ok(bits) = pack::<34>(&prim.pdu) else { return };
            let pdu_type = match mac::downlink::kind(bits.view(), prim.logical_channel == LogicalChannel::Stch) {
                Ok(kind) => kind,
                Err(error) => {
                    tracing::warn!("unsupported or invalid MAC PDU: {:?}", error);
                    return;
                }
            };
            let orig_start = prim.pdu.get_raw_start();
            match pdu_type {
                mac::downlink::PduKind::Resource => {
                    self.rx_mac_resource(queue, &mut message);
                }
                mac::downlink::PduKind::Fragment => {
                    self.rx_mac_frag(queue, &mut message);
                }
                mac::downlink::PduKind::End => {
                    self.rx_mac_end(queue, &mut message);
                }
                mac::downlink::PduKind::Sysinfo => {
                    self.rx_broadcast_sysinfo(queue, &mut message);
                }
                mac::downlink::PduKind::AccessDefine => {
                    self.rx_broadcast_access_define(&mut message);
                }
            }

            // Check if end of message reached by re-borrowing inner
            // If start was not updated, we also consider it end of message
            // If 16 or more bits remain (len of null pdu), we continue parsing
            if let SapMsgInner::TmvUnitdataInd(prim) = &message.msg {
                let consumed = (prim.pdu.get_raw_start() - orig_start) as u16;
                if cursor.advance(consumed).unwrap_or(false) {
                    tracing::trace!(
                        "rx_tmv_unitdata_ind_sch: Remaining {} bits: {:?}",
                        prim.pdu.get_len_remaining(),
                        prim.pdu.dump_bin_full(true)
                    );
                } else {
                    tracing::trace!("rx_tmv_unitdata_ind_sch: End of message reached");
                    break;
                }
            }
        }
    }

    // Parses an ACCESS-DEFINE PDU (ETSI TS 100 392-2 cl. 21.4.4.3) and adopts
    // the random access parameters for the access code it defines
    // (cl. 23.5.1.4.1).
    fn rx_broadcast_access_define(&mut self, message: &mut SapMsg) {
        tracing::trace!("rx_broadcast_access_define");
        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };

        let bits = match pack::<34>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("Invalid ACCESS-DEFINE storage: {:?}", error);
                return;
            }
        };
        let definition = match mac::downlink::access_define(bits.view()) {
            Ok(definition) => definition,
            Err(error) => {
                tracing::warn!("Invalid ACCESS-DEFINE: {:?}", error);
                return;
            }
        };

        // An MS on its common control channel ignores ACCESS-DEFINE PDUs marked
        // for an assigned control channel (cl. 23.5.1.4.1). The MS currently
        // only camps on the common control channel.
        let Some((code, params)) = definition.for_common_channel() else {
            tracing::trace!("rx_broadcast_access_define: ignoring assigned-control ACCESS-DEFINE");
            return;
        };
        self.access_params.update_access_define(&AccessDefine {
            common_or_assigned_control: false,
            access_code: code as u8,
            imm: params.imm,
            wt: params.wt,
            nu: params.nu,
            frame_len_factor: params.fl_factor,
            ts_pointer: params.ts_ptr,
            min_pdu_prio: params.min_pdu_prio,
            opt_field_flag: if params.subscriber_class.is_some() {
                1
            } else if params.gssi.is_some() {
                2
            } else {
                0
            },
            subscriber_class: params.subscriber_class,
            gssi: params.gssi,
        });
    }

    // Parses the sysinfo pdu
    fn rx_broadcast_sysinfo(&mut self, queue: &mut MessageQueue, message: &mut SapMsg) {
        tracing::trace!("rx_broadcast_sysinfo");
        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };

        let bits = match pack::<34>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("Invalid SYSINFO storage: {:?}", error);
                return;
            }
        };
        let pdu = match mac::downlink::sysinfo(bits.view()) {
            Ok(pdu) => pdu,
            Err(error) => {
                tracing::warn!("Invalid SYSINFO: {:?}", error);
                return;
            }
        };
        prim.pdu.seek(usize::from(pdu.sdu_offset));

        // Adopt the "default definition for access code A" if present
        // (cl. 21.4.4.1 / 23.5.1.4.10). Ignored by the store once a "common"
        // ACCESS-DEFINE for code A has been received.
        if let Some(def) = &pdu.default_access {
            self.access_params.update_sysinfo_default_a(
                &tetra_pdus::umac::fields::sysinfo_default_def_for_access_code_a::SysinfoDefaultDefForAccessCodeA {
                    imm: def.imm,
                    wt: def.wt,
                    nu: def.nu,
                    fl_factor: def.fl_factor,
                    ts_ptr: def.ts_ptr,
                    min_pdu_prio: def.min_pdu_prio,
                },
            );
        }

        // Record the serving cell's downlink main carrier (cl. 21.4.4) so a
        // received CHANNEL ALLOCATION element (cl. 21.5.2) can be classified as
        // same-carrier vs a different carrier. M2 acts only on same-carrier
        // assignments; cross-carrier retune is deferred to M3.
        let time_update = self.mac.adopt_sysinfo(&pdu);

        // Derive the uplink carrier from the cell's broadcast duplex parameters
        // and retune the transmitter if it changed (EN 300 392-2 cl. 18.4.2.2 /
        // cl. 21.4.4). The MS derives its uplink at camp from SYSINFO — not from
        // config — so the TX chain follows whatever cell it is actually on.
        self.maybe_retune_uplink(queue, &pdu);

        // TODO FIXME adopt sysinfo info into global state
        if let Some(new_time) = time_update {
            let m = SapMsg {
                sap: Sap::TmvSap,
                src: self.self_component,
                dest: TetraEntity::Lmac,
                msg: SapMsgInner::TmvConfigureReq(TmvConfigureReq {
                    time: Some(new_time),
                    ..Default::default()
                }),
            };
            tracing::debug!("rx_broadcast_sysinfo: Updated TdmaTime: {:?} -> {:?}", self.dltime, new_time);
            queue.push_back(m);
        }

        let tlsdu = BitBuffer::from_bitbuffer_pos(&prim.pdu);
        let m = SapMsg {
            sap: Sap::TlmbSap,
            src: TetraEntity::Umac,
            dest: TetraEntity::Mle,
            msg: SapMsgInner::TlmbSysinfoInd(TlmbSysinfoInd {
                endpoint_id: 0,
                tl_sdu: tlsdu,
                mac_broadcast_info: None,
            }),
        };

        queue.push_back(m);
    }

    /// Derive the uplink carrier from a decoded SYSINFO and, if it differs from
    /// the last derived value, request the PHY retune the transmitter
    /// (`TmvTxTuneReq` -> LMAC -> PHY).
    ///
    /// The uplink is resolved exactly as the codeplug math would (EN 300 392-2
    /// cl. 18.4.2.2 broadcast parameters / cl. 21.4.4 SYSINFO): the downlink is
    /// `100 MHz * band + 25 kHz * main_carrier + freq_offset`, and the duplex
    /// spacing is taken from the programmed [`DuplexTable`] for the broadcast
    /// duplex-spacing index (an operator override, e.g. a non-standard split,
    /// wins over the ETSI default). `reverse_operation` selects UL below/above
    /// DL. Deriving here — rather than seeding from config — means the MS always
    /// transmits on the uplink paired with the cell it actually camped on.
    fn maybe_retune_uplink(&mut self, queue: &mut MessageQueue, pdu: &mac::downlink::Sysinfo) {
        // Compute the uplink under the config guard, then drop the guard before
        // mutating self / queueing.
        let ul = {
            let cfg = self.config.config();
            if cfg.stack_mode != StackMode::Ms {
                return;
            }
            // Guard the value ranges the FreqInfo constructor asserts on, so a
            // malformed SYSINFO logs and is ignored instead of panicking.
            if pdu.freq_band > 8 || pdu.main_carrier >= 4000 {
                tracing::warn!(
                    "SYSINFO out-of-range RF params (band {}, carrier {}); not deriving uplink",
                    pdu.freq_band,
                    pdu.main_carrier
                );
                return;
            }
            let freq_offset_hz = match FreqInfo::freq_offset_id_to_hz(pdu.freq_offset_index) {
                Some(v) => v,
                None => {
                    tracing::warn!("SYSINFO invalid freq offset index {}", pdu.freq_offset_index);
                    return;
                }
            };
            match FreqInfo::from_components_with_table(
                pdu.freq_band,
                pdu.main_carrier,
                freq_offset_hz,
                pdu.reverse_operation,
                pdu.duplex_spacing,
                None,
                &cfg.duplex_table,
            ) {
                Ok(freq_info) => freq_info.get_freqs().1,
                Err(e) => {
                    tracing::warn!("Cannot derive uplink from SYSINFO: {}", e);
                    return;
                }
            }
        };

        if self.derived_ul_freq == Some(ul) {
            // Unchanged since the last SYSINFO — nothing to retune.
            return;
        }
        self.derived_ul_freq = Some(ul);
        tracing::info!(
            "UMAC: derived uplink {} Hz from SYSINFO (band {}, carrier {}, duplex idx {}, reverse {}); retuning TX",
            ul,
            pdu.freq_band,
            pdu.main_carrier,
            pdu.duplex_spacing,
            pdu.reverse_operation
        );
        queue.push_back(SapMsg {
            sap: Sap::TmvSap,
            src: self.self_component,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvTxTuneReq(TmvTxTuneReq { carrier_hz: ul }),
        });
    }

    fn rx_mac_resource(&mut self, queue: &mut MessageQueue, message: &mut SapMsg) {
        tracing::trace!("rx_mac_resource");
        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };
        assert!(prim.pdu.get_pos() == 0); // We should be at the start of the MAC PDU

        let bits = match pack::<34>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("Invalid MAC-RESOURCE storage: {:?}", error);
                return;
            }
        };
        let (filtered, own) = {
            let cfg = self.config.config();
            (
                cfg.stack_mode == StackMode::Ms && cfg.ms.is_some(),
                cfg.ms.as_ref().map_or(0, |ms| ms.issi),
            )
        };
        let event = match self.mac.resource_event(&mut self.random_access, bits.view(), own, filtered) {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!("Invalid MAC-RESOURCE: {:?}", error);
                return;
            }
        };
        let outcome = event.outcome;
        let pdu = outcome.header;

        // Feed the random access response detector (cl. 23.5.1.4.8): a
        // MAC-RESOURCE addressed to our ISSI with the random access flag set
        // acknowledges the request we sent, completing the access procedure.
        match event.access {
            mac::AccessCompletion::None => {}
            mac::AccessCompletion::Complete { receipt } => {
                self.finish_receipt(receipt, true);
            }
            mac::AccessCompletion::Fragment(action) => self.apply_fragment_action(queue, action),
        }

        let orig_end = prim.pdu.get_raw_end();
        prim.pdu.seek(usize::from(pdu.header_bits));
        prim.pdu.set_raw_end(prim.pdu.get_raw_start() + usize::from(outcome.payload.end));
        if outcome.second_half_stolen {
            self.signal_lmac_second_half_stolen(queue);
        }
        match outcome.route {
            mac::ResourceRoute::Drop(mac::DropReason::NoAddress | mac::DropReason::Encrypted) => return,
            mac::ResourceRoute::Drop(reason) => tracing::debug!("MAC-RESOURCE dropped: {:?}", reason),
            mac::ResourceRoute::FragmentStart => {
                self.defrag.insert_first(&mut prim.pdu, self.mac.dltime, pdu.addr.unwrap(), None);
            }
            mac::ResourceRoute::Deliver => {
                let m = SapMsg {
                    sap: Sap::TmaSap,
                    src: TetraEntity::Umac,
                    dest: TetraEntity::Llc,
                    msg: SapMsgInner::TmaUnitdataInd(TmaUnitdataInd {
                        pdu: Some(BitBuffer::from_bitbuffer_pos(&prim.pdu)),
                        main_address: pdu.addr.unwrap(),
                        scrambling_code: prim.scrambling_code,
                        endpoint_id: 0,        // TODO FIXME
                        new_endpoint_id: None, // TODO FIXME
                        css_endpoint_id: None, // TODO FIXME
                        air_interface_encryption: pdu.encryption_mode as Todo,
                        chan_change_response_req: false,
                        chan_change_handle: None,
                        chan_info: None,
                    }),
                };
                queue.push_back(m);
            }
            mac::ResourceRoute::Empty => tracing::info!("rx_mac_resource: empty PDU not passed to LLC"),
        }

        // Since this is not a null pdu, more MAC PDUs may follow
        // This allows parent function to continue parsing
        prim.pdu.set_raw_end(orig_end);
        prim.pdu.set_raw_pos(prim.pdu.get_raw_start() + usize::from(outcome.payload.next));
        prim.pdu.set_raw_start(prim.pdu.get_raw_pos());
    }

    /// Tell LMAC that this slot's Block2 is also stolen for associated
    /// signalling (STCH, not TCH/S speech), so it is decoded on the control-plane
    /// chain rather than the speech path (cl. 9.4.4.3.2). Immediate priority so
    /// LMAC applies it before classifying the Block2 burst. Mirrors
    /// `UmacBs::signal_lmac_second_half_stolen`.
    fn signal_lmac_second_half_stolen(&mut self, queue: &mut MessageQueue) {
        let m = SapMsg {
            sap: Sap::TmvSap,
            src: TetraEntity::Umac,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvConfigureReq(TmvConfigureReq {
                blk2_stolen: Some(true),
                ..Default::default()
            }),
        };
        queue.push_prio(m, MessagePrio::Immediate);
    }

    fn rx_mac_frag(&mut self, _queue: &mut MessageQueue, message: &mut SapMsg) {
        tracing::trace!("rx_mac_frag");
        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };
        assert!(prim.pdu.get_pos() == 0); // We should be at the start of the MAC PDU

        let bits = match pack::<34>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("Invalid MAC-FRAG storage: {:?}", error);
                return;
            }
        };
        let payload = match mac::downlink::fragment(bits.view()) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!("Invalid MAC-FRAG: {:?}", error);
                return;
            }
        };
        prim.pdu.seek(usize::from(payload.start));
        prim.pdu.set_raw_end(prim.pdu.get_raw_start() + usize::from(payload.end));

        // Decrypt if needed
        if let Some(_aie_info) = self.defrag.get_aie_info(self.mac.dltime) {
            // TODO FIXME implement
            unimplemented_log!("rx_mac_frag: Encryption not supported");
            return;
        }

        // Insert into defragmenter
        self.defrag.insert_next(&mut prim.pdu, self.mac.dltime);
    }

    fn rx_mac_end(&mut self, queue: &mut MessageQueue, message: &mut SapMsg) {
        tracing::trace!("rx_mac_end");
        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };
        assert!(prim.pdu.get_pos() == 0); // We should be at the start of the MAC PDU

        let bits = match pack::<34>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("Invalid MAC-END storage: {:?}", error);
                return;
            }
        };
        let payload = match mac::downlink::end(bits.view()) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!("Invalid MAC-END: {:?}", error);
                return;
            }
        };
        let orig_end = prim.pdu.get_raw_end();
        prim.pdu.seek(usize::from(payload.start));
        prim.pdu.set_raw_end(prim.pdu.get_raw_start() + usize::from(payload.end));

        // Decrypt if needed
        if let Some(_aie_info) = self.defrag.get_aie_info(self.mac.dltime) {
            // TODO FIXME implement
            unimplemented!("rx_mac_end: Encryption not supported");
            // TODO FIXME Also re-parse chanalloc
        }

        // Insert into defragmenter
        self.defrag.insert_last(&mut prim.pdu, self.mac.dltime);

        // Fetch finalized block
        let defragbuf = self.defrag.take_defragged_buf(self.mac.dltime);
        let Some(defragbuf) = defragbuf else {
            tracing::warn!("rx_mac_end: could not obtain defragged buf");
            return;
        };

        // Pass block directly to LLC
        tracing::debug!("rx_mac_end: sdu: {:?}", defragbuf.buffer.dump_bin());

        // MS receive filter (ETSI TS 100 392-2 clause 23 addressing): drop a
        // reassembled message addressed to another subscriber/group. The first
        // fragment is normally already filtered in rx_mac_resource, so this is
        // a defensive backstop. Advance the parse position so any following
        // MAC PDU in the block is still parsed.
        if !self.accept_downlink_address(&defragbuf.addr) {
            tracing::debug!("rx_mac_end: dropping reassembled PDU addressed to {} (not this MS)", defragbuf.addr);
            prim.pdu.set_raw_end(orig_end);
            prim.pdu.set_raw_pos(prim.pdu.get_raw_start() + usize::from(payload.next));
            prim.pdu.set_raw_start(prim.pdu.get_raw_pos());
            return;
        }

        let m = SapMsg {
            sap: Sap::TmaSap,
            src: TetraEntity::Umac,
            dest: TetraEntity::Llc,
            msg: SapMsgInner::TmaUnitdataInd(TmaUnitdataInd {
                pdu: Some(defragbuf.buffer),
                main_address: defragbuf.addr,
                scrambling_code: prim.scrambling_code,
                endpoint_id: 0,              // TODO FIXME
                new_endpoint_id: None,       // TODO FIXME
                css_endpoint_id: None,       // TODO FIXME
                air_interface_encryption: 0, // TODO FIXME implement
                chan_change_response_req: false,
                chan_change_handle: None,
                chan_info: None,
            }),
        };
        queue.push_back(m);

        // Since this is not a null pdu, more MAC PDUs may follow
        // This allows parent function to continue parsing
        prim.pdu.set_raw_end(orig_end);
        prim.pdu.set_raw_pos(prim.pdu.get_raw_start() + usize::from(payload.next));
        prim.pdu.set_raw_start(prim.pdu.get_raw_pos());
    }

    pub fn rx_tmv_aach(&mut self, queue: &mut MessageQueue, mut message: SapMsg) {
        tracing::trace!("rx_tmv_aach");

        // TODO FIXME, more extensively store and process AACH state in both LMAC and UMAC
        // Then we send a msg down only if a change is needed, like we do for the scrambling code

        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };

        // Reset the per-slot downlink usage marker: it describes only this slot
        // (cl. 21.4.7.2) and gates this slot's TCH/S relay in `rx_tmd_prim`. A
        // stale value must never leak onto a later slot.
        let bits = match pack::<34>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("Invalid ACCESS-ASSIGN storage: {:?}", error);
                return;
            }
        };
        let assignment = match self.mac.aach(bits.view()) {
            Ok(assignment) => assignment,
            Err(error) => {
                tracing::warn!("Invalid ACCESS-ASSIGN: {:?}", error);
                return;
            }
        };

        let m = SapMsg {
            sap: Sap::TmvSap,
            src: TetraEntity::Umac,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvConfigureReq(TmvConfigureReq {
                is_traffic: Some(assignment.traffic_marker.is_some()),
                ..Default::default()
            }),
        };
        // This message needs to be processed NOW since it affects the other blocks in this timeslot
        queue.push_prio(m, MessagePrio::Immediate);

        // Drive the MS random access state machine against this slot's ACCESS-ASSIGN.
        if let Some(access) = assignment.access {
            self.drive_random_access_assign(queue, &access);
        }
    }

    /// Advance the random access state machine (cl. 23.5.1.4) for one downlink
    /// slot's ACCESS-ASSIGN and act on the resulting decision. No-op unless a
    /// random access attempt is currently in progress.
    #[cfg(test)]
    fn drive_random_access(&mut self, queue: &mut MessageQueue, aa: &AccessAssign) {
        let assign = interpret_access_assign(aa, true);
        self.drive_random_access_assign(queue, &assign);
    }

    fn drive_random_access_assign(&mut self, queue: &mut MessageQueue, assign: &tetra_ms_core::random_access::SlotAccessAssign) {
        // If the previously active uplink transfer has completed (acknowledged
        // or abandoned), pull the next queued transfer into the active slots so
        // it can contend for access. Done first, every slot, so a transfer that
        // was held behind another (cl. 23.5.1.4 permits only one at a time) is
        // never starved.
        // Access code A is the default code available to all MSs (SYSINFO
        // default-A, cl. 21.4.4.1). Without advertised parameters we cannot
        // legally access, so keep any queued uplink waiting until the BS
        // broadcasts them (they arrive shortly after camping, via SYSINFO
        // default-A or ACCESS-DEFINE).
        let code = AccessCode::A;
        let params = self.access_params.params_for(code).cloned();

        // Start the random access procedure for a queued uplink block that has
        // not been initiated yet (cl. 23.5.1.4). Initiation is deferred here
        // from `rx_tma_prim` so an uplink queued before the access parameters
        // were advertised is transmitted as soon as they arrive, rather than
        // dropped. No-op when nothing is queued and no attempt is in progress.
        match self.mac.begin_access(&mut self.random_access, params.as_ref()) {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => {
                tracing::warn!("drive_random_access: not initiated: {:?}", error.reason);
                for handle in [error.first_receipt, error.fragment_receipt] {
                    self.finish_receipt(handle, false);
                }
                return;
            }
        }
        let params = params.expect("core verified access parameters present");

        // The ACCESS-ASSIGN on the AACH designates the access rights of the
        // uplink subslots of the slot two timeslots later (cl. 23.5.1.4.2). We
        // are camped on the common control channel, where the per-slot AACH
        // designation is authoritative, so any slot carrying a valid
        // ACCESS-ASSIGN is treated as a potential opportunity (ul_slot_valid).
        let mut rng = ThreadRaRng;
        let action = self.random_access.poll_downlink_slot(self.dltime, assign, true, &params, &mut rng);

        match action {
            Some(RaAction::Transmit { ul_time, subslot }) => {
                self.emit_uplink(queue, ul_time, subslot);
            }
            Some(RaAction::Failed(f)) => {
                tracing::warn!("random access abandoned: {:?}", f);
                // The MS-MAC exhausted its access attempts without the BS
                // acknowledging (cl. 23.5.1.4.9). Signal the LLC that the MAC
                // could not deliver this TM-SDU so the acknowledged basic link
                // retransmits it (cl. 22.3.2.3); if a fragmented transfer, the
                // receipt lives on the fragment.
                let (first, tail) = self.mac.abandon_access();
                for handle in [first, tail] {
                    self.finish_receipt(handle, false);
                }
            }
            Some(RaAction::Succeeded) | None => {}
        }
    }

    /// Emit the queued uplink MAC block to LMAC for transmission on the granted
    /// uplink slot. The block is kept in `pending_uplink` so it can be
    /// retransmitted if the random access attempt has to retry (cl. 23.5.1.4.7).
    fn emit_uplink(&mut self, queue: &mut MessageQueue, ul_time: TdmaTime, subslot: Subslot) {
        let Some(pending) = self.pending_uplink.as_ref() else {
            tracing::warn!("random access requested transmit but no pending uplink block");
            return;
        };

        tracing::info!("random access: transmitting MAC-ACCESS at UL {:?} subslot {:?}", ul_time, subslot);

        let blk = TmvUnitdataReq {
            mac_block: unpack(&pending.mac_block),
            logical_channel: desktop_channel(pending.logical_channel),
            scrambling_code: pending.scrambling_code,
        };

        let m = SapMsg {
            sap: Sap::TmvSap,
            src: self.self_component,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvUnitdataReq(TmvUnitdataReqSlot {
                ts: ul_time,
                ul_phy_chan: PhysicalChannel::Cp,
                placement: match subslot {
                    Subslot::One => TxBurstPlacement::Subslot1,
                    Subslot::Two => TxBurstPlacement::Subslot2,
                },
                blk1: Some(blk),
                blk2: None,
                bbk: None,
                // Contention access uses the exact selected opportunity.
                reserved_access: false,
            }),
        };
        queue.push_back(m);
    }

    /// Emit the fragment-end PDU that completes a fragmented uplink transfer, on
    /// the capacity the base station granted in `grant` (ETSI TS 100 392-2
    /// cl. 23.5.2 basic slot granting; cl. 23.4.2.1.2 uplink fragmentation
    /// form i). This is a **reserved-access** transmission (the BS reserved the
    /// slot in response to our frag-start capacity request), so it does not
    /// contend via the random access algorithm. Consumes `pending_fragment`.
    ///
    /// The fragment-end PDU and channel are chosen by the remainder size at
    /// frag-start time ([`FragEndKind`]) and must match the grant:
    /// - `Req1Subslot` -> `FirstSubslotGranted` -> MAC-END-HU on SCH/HU (Control
    ///   Uplink Burst) — remainder up to ~81 TM-SDU bits;
    /// - `Req1Slot` -> `Grant1Slot` -> MAC-END-UL on SCH/F (Normal Uplink Burst)
    ///   — remainder up to ~254 TM-SDU bits;
    /// - `Req{N}Slots` -> `Grant{N}Slots` -> `N-1` MAC-FRAG-UL continuations plus
    ///   a terminating MAC-END-UL across N reserved SCH/F full slots (true
    ///   multi-slot fragmentation) — driven out one block per slot by
    ///   [`Self::drive_reserved_tx`].
    ///
    /// Only the "next opportunity" granting delay (cl. 23.5.2.2.2) is handled;
    /// any other capacity allocation / granting delay, or a grant that does not
    /// match the requested [`FragEndKind`], is logged and the remainder dropped
    /// (the MM layer retransmits the whole demand). For "capacity allocation at
    /// next opportunity" the reserved slot is the same-numbered uplink timeslot
    /// in the same TDMA frame as the downlink slot carrying the grant; since the
    /// uplink frame is a fixed 2-timeslot delay from the downlink (cl. 9.3.9
    /// Frame alignment), that is `dltime + 2`.
    #[cfg(test)]
    fn emit_fragment_end(&mut self, queue: &mut MessageQueue, grant: Option<&BasicSlotgrant>) {
        let grant = grant.map(|grant| Grant {
            next_opportunity: grant.granting_delay == BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
            capacity: match grant.capacity_allocation {
                BasicSlotgrantCapAlloc::FirstSubslotGranted => GrantCapacity::FirstSubslot,
                BasicSlotgrantCapAlloc::SecondSubslotGranted => GrantCapacity::SecondSubslot,
                other => GrantCapacity::Slots(other.to_req_slotcount().try_into().expect("slot grant fits u8")),
            },
        });
        self.emit_fragment_end_core(queue, grant);
    }

    #[cfg(test)]
    fn emit_fragment_end_core(&mut self, queue: &mut MessageQueue, grant: Option<Grant>) {
        if let Some(action) = self.mac.grant_fragment(grant) {
            self.apply_fragment_action(queue, action);
        }
    }

    fn apply_fragment_action(&mut self, queue: &mut MessageQueue, action: FragmentAction<u32>) {
        match action {
            FragmentAction::Rejected { reason, receipt } => {
                tracing::warn!("uplink fragmentation rejected: {:?}", reason);
                self.finish_receipt(receipt, false);
            }
            FragmentAction::Reserved => self.drive_reserved_tx(queue),
            FragmentAction::Transmit(burst) => self.emit_core_reserved(queue, burst),
        }
    }

    fn emit_core_reserved(&mut self, queue: &mut MessageQueue, burst: mac::Burst<u32>) {
        let block = TmvUnitdataReq {
            mac_block: unpack(&burst.block),
            logical_channel: desktop_channel(burst.channel),
            scrambling_code: burst.scrambling_code,
        };
        queue.push_back(SapMsg {
            sap: Sap::TmvSap,
            src: self.self_component,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvUnitdataReq(TmvUnitdataReqSlot {
                ts: burst.time,
                ul_phy_chan: PhysicalChannel::Cp,
                placement: match burst.channel {
                    Channel::SchHu => TxBurstPlacement::Subslot1,
                    _ => TxBurstPlacement::FullSlot,
                },
                blk1: Some(block),
                blk2: None,
                bbk: None,
                reserved_access: true,
            }),
        });
        self.finish_receipt(burst.receipt, true);
    }

    /// Drive an in-flight multi-slot reserved uplink transfer (ETSI TS 100 392-2
    /// cl. 23.4.2.1.2 / 23.5.2). Called each downlink slot (from `tick_start`,
    /// and once from `emit_fragment_end` when the plan is created): when the
    /// front reserved slot's uplink time (`dltime + 2`) is reached, the next
    /// prebuilt full-slot block (a MAC-FRAG-UL continuation, or the final
    /// MAC-END-UL) is emitted to LMAC as a reserved-access burst that the PHY
    /// transmits at exactly that slot.
    ///
    /// The reserved slots are spaced one TDMA frame apart (>= 4 timeslots) and
    /// the downlink clock advances one slot per tick, so at most one block is
    /// ever due in a given tick. If a reserved slot is missed (we advanced past
    /// it without transmitting — e.g. the downlink clock was re-seeded by a SYNC
    /// burst), the whole plan is abandoned and its transmit receipt discarded so
    /// the LLC/MM retransmit the whole transfer.
    fn drive_reserved_tx(&mut self, queue: &mut MessageQueue) {
        let action = self.mac.reserved();
        self.apply_reserved_action(queue, action);
    }

    fn apply_reserved_action(&mut self, queue: &mut MessageQueue, action: ReservedAction<u32>) {
        match action {
            ReservedAction::Waiting => {}
            ReservedAction::Missed { receipt } => {
                tracing::warn!("uplink fragmentation: missed reserved slot; abandoning transfer");
                self.finish_receipt(receipt, false);
            }
            ReservedAction::Transmit(burst) => self.emit_core_reserved(queue, burst),
        }
    }

    /// Emit the MS uplink burst on the assigned traffic slot for this timeslot:
    /// continuous TCH/S speech while this MS is the talker, and/or a stolen
    /// half-slot (FACCH/STCH, cl. 23) carrying associated signalling.
    ///
    /// Mirrors the BS downlink traffic emitter (`lmac_bs` NDB path) for the
    /// uplink direction. UMAC is the transmit timing authority (ETSI TS 100
    /// 392-2 cl. 23): once per timeslot it decides whether a burst is due and
    /// clocks exactly one down to the LMAC, which channel-codes it into a Normal
    /// Uplink Burst (cl. 9.4.4.2). The speech stream is supplied by CC-MS (the
    /// U-plane owner, cl. 14.5.1.4) via the jitter buffer; on underrun a silence
    /// frame is emitted so a granted slot is never left dark mid-call.
    ///
    /// A burst is emitted on the paired uplink slot (`dltime + 2`, cl. 9.3.9)
    /// only when all of the following hold:
    /// - the slot is one the network assigned to us as a traffic channel
    ///   (`assigned_traffic_slots`, cl. 21.5.2) — never hardcoded to a fixed
    ///   timeslot — and it is not the frame-18 control frame (cl. 9.5.1c);
    /// - a valid serving-cell scrambling code is known (we are camped); and
    /// - either we hold the transmission grant (`uplink_tx_granted`, the talker,
    ///   cl. 14.5.1.4) — emitting speech — or associated signalling is queued to
    ///   steal a half-slot (e.g. a U-TX-DEMAND floor request while listening).
    ///   With neither, a non-talking party stays silent.
    ///
    /// When both hold (talking and a signalling PDU is queued) the slot is
    /// stolen: the STCH signalling block takes the first half and the TCH/S
    /// speech the second (normal training sequence 2, cl. 9.4.4.3.2).
    #[cfg(test)]
    fn drive_uplink_traffic(&mut self, queue: &mut MessageQueue) {
        let Some(burst) = self.mac.traffic() else {
            return;
        };
        self.emit_core_traffic(queue, burst);
    }

    fn emit_core_traffic(&mut self, queue: &mut MessageQueue, burst: mac::TrafficBurst<u32>) {
        tracing::info!(
            ul = %burst.time,
            dltime = %self.dltime,
            assigned_ts = ?self.assigned_traffic_slots,
            granted = self.uplink_tx_granted,
            has_steal = burst.stolen.is_some(),
            speech_bits = burst.speech.len(),
            "MS uplink traffic: emitting TCH/S burst"
        );

        let blk = TmvUnitdataReq {
            mac_block: unpack(&burst.speech),
            logical_channel: LogicalChannel::TchS,
            scrambling_code: burst.scrambling_code,
        };

        // If associated signalling is queued for FACCH/STCH stealing (cl. 23),
        // steal the first half of this traffic slot: blk1 carries the STCH
        // signalling block, blk2 the remaining TCH/S speech half. The burst then
        // uses normal training sequence 2 (selected in the LMAC from the STCH
        // logical channel). Otherwise the whole slot carries TCH/S speech (blk1
        // only). Mark the stolen block's LLC receipt transmitted so the
        // acknowledged-mode basic link progresses (cl. 22.3.2.3).
        let (blk1, blk2) = match burst.stolen {
            Some(steal) => {
                self.finish_receipt(steal.tx_reporter, true);
                let stch = TmvUnitdataReq {
                    mac_block: unpack(&steal.mac_block),
                    logical_channel: LogicalChannel::Stch,
                    scrambling_code: steal.scrambling_code,
                };
                (stch, Some(blk))
            }
            None => (blk, None),
        };

        let m = SapMsg {
            sap: Sap::TmvSap,
            src: self.self_component,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvUnitdataReq(TmvUnitdataReqSlot {
                ts: burst.time,
                // Traffic physical channel carries the TCH/S burst (cl. 9.4.4.2),
                // matching the BS which uses Tp for traffic and Cp otherwise.
                ul_phy_chan: PhysicalChannel::Tp,
                placement: TxBurstPlacement::FullSlot,
                blk1: Some(blk1),
                blk2,
                bbk: None,
                // Continuous traffic is clock-driven on the assigned slot, not a
                // BS-reserved random-access opportunity (cl. 23.5.2); the PHY may
                // schedule it at the paired uplink slot like BS downlink traffic.
                reserved_access: false,
            }),
        };
        queue.push_back(m);
    }

    pub fn rx_tmv_bsch(&mut self, queue: &mut MessageQueue, mut message: SapMsg) {
        tracing::trace!("rx_tmv_bsch");
        let SapMsgInner::TmvUnitdataInd(prim) = &mut message.msg else {
            panic!()
        };

        let bits = match pack::<34>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("Invalid MAC-SYNC storage: {:?}", error);
                return;
            }
        };
        let pdu = match mac::downlink::sync(bits.view()) {
            Ok(pdu) => pdu,
            Err(error) => {
                tracing::warn!("Invalid MAC-SYNC: {:?}", error);
                return;
            }
        };
        prim.pdu.seek(usize::from(pdu.sdu_offset));

        // Adopt the colour code. Together with the MCC/MNC provided by MLE over
        // TLMC, it derives the scrambling code (cl. 23.2.2 / 8.2.5).
        if self.cc != Some(pdu.colour_code) {
            tracing::info!("rx_tmv_bsch: colour code {:?} -> {}", self.cc, pdu.colour_code);
        }

        // Seed the absolute downlink time from the SYNC burst (cl. 7 / 21.4.4.2).
        // UMAC free-runs this between SYNC bursts (see `tick_start`) and re-seeds
        // it here each frame 18.
        self.mac.adopt_sync(pdu);

        // Push the recovered time down to LMAC so it classifies logical channels
        // (BNCH / frame 18) and interprets AACH against the correct absolute time.
        let m = SapMsg {
            sap: Sap::TmvSap,
            src: self.self_component,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvConfigureReq(TmvConfigureReq {
                time: Some(pdu.time),
                ..Default::default()
            }),
        };
        queue.push_back(m);

        // Forward the remaining bits (the D-MLE-SYNC SDU, cl. 18.4.2.1) up to MLE
        // for initial cell selection (cl. 18.3.4.6). MLE replies with a
        // TL-CONFIGURE carrying the valid MCC/MNC, which lets us derive the
        // scrambling code and submit it to LMAC.
        let tlsdu = BitBuffer::from_bitbuffer_pos(&prim.pdu);
        let m = SapMsg {
            sap: Sap::TlmbSap,
            src: TetraEntity::Umac,
            dest: TetraEntity::Mle,
            msg: SapMsgInner::TlmbSyncInd(TlmbSyncInd {
                endpoint_id: 0,
                tl_sdu: tlsdu,
            }),
        };
        queue.push_back(m);
    }

    fn rx_tma_prim(&mut self, _queue: &mut MessageQueue, message: SapMsg) {
        tracing::trace!("rx_tma_prim");
        let SapMsgInner::TmaUnitdataReq(mut prim) = message.msg else {
            panic!("rx_tma_prim: unexpected primitive");
        };

        let bits = match pack::<512>(&prim.pdu) {
            Ok(bits) => bits,
            Err(error) => {
                tracing::warn!("rx_tma_prim: SDU exceeds portable MAC capacity: {:?}", error);
                if let Some(reporter) = prim.tx_reporter {
                    reporter.try_mark_discarded();
                }
                return;
            }
        };
        let issi = self.config.config().ms.as_ref().map(|ms| ms.issi);
        let Some(issi) = issi else {
            tracing::warn!("rx_tma_prim: MS identities not configured; dropping uplink");
            if let Some(reporter) = prim.tx_reporter {
                reporter.try_mark_discarded();
            }
            return;
        };
        let receipt = match self.track_reporter(prim.tx_reporter.take()) {
            Ok(receipt) => receipt,
            Err(reporter) => {
                tracing::warn!("rx_tma_prim: desktop receipt handles exhausted; dropping uplink");
                reporter.try_mark_discarded();
                return;
            }
        };
        let request = mac::SubmitRequest {
            issi,
            sdu: bits.view(),
            stealing: prim.stealing_permission,
            priority: prim.pdu_priority,
            emergency: prim.is_emergency,
            receipt,
        };
        let active = self.random_access.is_active();
        let result = match self.mac.submit_sdu(request, active) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!("rx_tma_prim: portable MAC rejected SDU: {:?}", error.reason);
                self.finish_receipt(error.receipt, false);
                return;
            }
        };
        tracing::debug!("rx_tma_prim: queued SDU via {:?}", result.route);
        if let Some(dropped) = result.discarded {
            tracing::warn!("rx_tma_prim: uplink queue full; dropping oldest transfer");
            self.discard_queued_uplink(&dropped);
        }
        self.finish_receipt(result.discarded_stolen.and_then(|block| block.tx_reporter), false);

        // Queue for transmission at the next valid random-access opportunity.
        // The MS-MAC random access procedure (cl. 23.5.1.4) — access-frame
        // selection and the randomised access algorithm — is initiated and
        // driven by `drive_random_access` on each downlink slot carrying a
        // valid ACCESS-ASSIGN, once access parameters for code A have been
        // advertised (SYSINFO default-A / ACCESS-DEFINE, cl. 21.4.4.1). Holding
        // the block here rather than requiring the parameters to already be
        // present avoids dropping the first uplink when the trigger — e.g. MM
        // registration on cell selection — fires before the broadcast carrying
        // the access parameters has been received. The actual transmit is
        // emitted to LMAC and PHY from `drive_random_access`. If the SDU was
        // fragmented, the MAC-END-HU remainder is emitted from `rx_mac_resource`
        // when the BS grants a subslot in response to the frag-start.
    }

    /// Mark every transmit receipt carried by a queued (never-transmitted)
    /// uplink as discarded (cl. 22.3.2.3) so the LLC retransmits it. Used when a
    /// queued transfer is dropped on overflow without ever reaching the air.
    fn discard_queued_uplink(&mut self, dropped: &QueuedUplink) {
        self.finish_receipt(dropped.pending.tx_reporter, false);
        if let Some(fragment) = dropped.fragment.as_ref() {
            self.finish_receipt(fragment.tx_reporter, false);
        }
    }

    /// Promote the next queued uplink transfer (if any) into the active
    /// `pending_uplink`/`pending_fragment` slots, but only when nothing is
    /// currently contending for random access. Called each downlink slot from
    /// `drive_random_access`, this is what lets a transfer queued behind another
    /// (e.g. a group-attach behind an LLC BL-ACK) start once the earlier one has
    /// been acknowledged or abandoned.
    #[cfg(test)]
    fn promote_next_uplink(&mut self) {
        self.mac.promote(self.random_access.is_active());
    }

    /// Build a 124-bit STCH type-1 MAC block wrapping `sdu` (an LLC BL-DATA PDU)
    /// in a MAC-DATA header (ETSI TS 100 392-2 cl. 21.4.3.3) for FACCH/STCH
    /// half-slot stealing on the granted uplink traffic channel (cl. 23).
    ///
    /// Mirrors the BS downlink FACCH builder (`rx_ul_tma_unitdata_req` STCH path)
    /// but for the uplink direction, where the MS uses MAC-DATA rather than the
    /// downlink-only MAC-RESOURCE. The length indication covers header + SDU +
    /// sub-octet fill (cl. 23.4.2.2); bits beyond `length_ind` octets are ignored
    /// by the receiver. Returns `None` if the PDU does not fit a single STCH
    /// half-slot, in which case the caller falls back to MCCH random access.
    #[cfg(test)]
    fn build_stch_mac_data_block(issi: u32, sdu: &mut BitBuffer) -> Option<BitBuffer> {
        let bits = pack::<512>(sdu).ok()?;
        let block = mac::wire::stolen_data(issi, bits.view()).ok()?;
        sdu.seek(sdu.get_len());
        Some(unpack(&block))
    }

    /// Build a 92-bit MAC-ACCESS type-1 block addressed with the MS's ISSI
    /// (ETSI TS 100 392-2 cl. 21.4.2.1). Returns `None` if the SDU needs
    /// fragmentation. No length indication is used: a self-contained PDU
    /// fills the subslot, with a one-bit delimiter then zero fill after the SDU
    /// (cl. 23.4.2.2 / 23.4.3.2).
    pub fn build_mac_access_block(issi: u32, sdu: &mut BitBuffer) -> Option<BitBuffer> {
        let bits = pack::<512>(sdu).ok()?;
        let block = mac::wire::access(issi, bits.view()).ok()?;
        sdu.seek(sdu.get_len());
        Some(unpack(&block))
    }

    /// Largest TM-SDU remainder (in bits) that a single MAC-END-UL can carry on
    /// an SCH/F full slot: header + SDU + fill must be a whole number of octets
    /// not exceeding the 268-bit block (cl. 21.4.2.5 / 23.4.2.2).
    #[cfg(test)]
    const fn max_end_ul_sdu_bits() -> usize {
        mac::wire::END_UL_SDU_BITS as usize
    }

    /// TM-SDU bits carried by a single MAC-FRAG-UL continuation: the whole
    /// 268-bit SCH/F block minus the 4-bit header (cl. 21.4.2.4). A continuation
    /// fills its slot exactly (no fill bits, no length indication — the length
    /// is carried by the terminating MAC-END-UL).
    #[cfg(test)]
    const fn max_frag_ul_sdu_bits() -> usize {
        mac::wire::FRAG_UL_SDU_BITS as usize
    }

    /// Number of full SCH/F slots a multi-slot uplink fragment remainder needs:
    /// `total_slots - 1` MAC-FRAG-UL continuations of [`Self::max_frag_ul_sdu_bits`]
    /// bits each plus a terminating MAC-END-UL of at most
    /// [`Self::max_end_ul_sdu_bits`] bits (ETSI TS 100 392-2 cl. 23.4.2.1.2).
    /// Returns `None` if the remainder needs more than [`MAX_UL_FRAG_SLOTS`]
    /// slots. The caller only reaches this for a remainder that does not fit a
    /// single MAC-END-UL, so `total_slots >= 2`.
    #[cfg(test)]
    fn compute_ul_frag_slots(remainder_len: usize) -> Option<usize> {
        mac::wire::fragment_slots(remainder_len.try_into().ok()?).ok().map(usize::from)
    }

    /// Build one MAC-FRAG-UL continuation block (ETSI TS 100 392-2 cl. 21.4.2.4)
    /// on a reserved SCH/F full slot: the 4-bit header followed by exactly
    /// [`mac::wire::FRAG_UL_SDU_BITS`] TM-SDU bits, filling the 268-bit block with
    /// no fill bits and no length indication (the terminating MAC-END-UL carries
    /// the overall length). `sdu_chunk` must hold exactly that many bits.
    pub fn build_mac_frag_ul_block(sdu_chunk: &mut BitBuffer) -> BitBuffer {
        let bits = pack::<34>(sdu_chunk).expect("MAC-FRAG-UL chunk exceeds capacity");
        let block = mac::wire::fragment(bits.view()).expect("MAC-FRAG-UL requires 264 SDU bits");
        sdu_chunk.seek(sdu_chunk.get_len());
        unpack(&block)
    }

    /// Split a multi-slot fragment `remainder` into `total_slots` full-slot
    /// blocks: `total_slots - 1` MAC-FRAG-UL continuations followed by a
    /// terminating MAC-END-UL (ETSI TS 100 392-2 cl. 23.4.2.1.2). Returns the
    /// blocks in transmission order, or `None` if the sizing is inconsistent
    /// (should not happen for a `total_slots` from [`mac::wire::fragment_slots`]).
    pub fn build_ul_frag_blocks(remainder: &mut BitBuffer, total_slots: usize) -> Option<Vec<BitBuffer>> {
        let bits = pack::<512>(remainder).ok()?;
        let blocks = mac::wire::FragmentBlocks::build(bits.view(), total_slots.try_into().ok()?).ok()?;
        remainder.seek(remainder.get_len());
        Some(blocks.as_slice().iter().map(unpack).collect())
    }

    /// The sequence of reserved uplink slots for a multi-slot transfer, starting
    /// at `first` (the `dltime + 2` uplink slot paired with the downlink slot
    /// carrying the grant, cl. 9.3.9 / 23.5.2.2.2). Subsequent slots are the
    /// same-numbered timeslot in each following TDMA frame (step +4 timeslots),
    /// skipping any mandatory CLCH slot — mirroring the BS
    /// `ul_find_grant_opportunity` "capacity allocation at next opportunity"
    /// stepping so the MS reproduces the BS's reserved set exactly.
    #[cfg(test)]
    fn reserved_slot_sequence(first: TdmaTime, total_slots: usize) -> VecDeque<TdmaTime> {
        mac::reserved_slot_sequence(first, total_slots.try_into().expect("reserved slot count fits u8"))
            .expect("reserved slot count or time invalid")[..total_slots]
            .iter()
            .copied()
            .collect()
    }

    /// Build the first fragment of an oversized uplink TM-SDU: a MAC-ACCESS
    /// "start of fragmentation" block (ETSI TS 100 392-2 cl. 21.4.2.1,
    /// cl. 23.4.2.1.2). The block is addressed with the MS's own ISSI and
    /// carries a **capacity request** — the fragmentation flag set plus a
    /// "reservation requirement" sized to the remainder — so the base station
    /// grants uplink capacity for the remainder (cl. 23.5.2): one subslot
    /// (`Req1Subslot`) for a MAC-END-HU remainder, or one full slot
    /// (`Req1Slot`) for a MAC-END-UL remainder. The first fragment carries as
    /// many TM-SDU bits as exactly fill the MAC block after the header, so there
    /// are no fill bits (cl. 23.4.2.1.2).
    ///
    /// Returns the full 92-bit type-1 block, a `BitBuffer` holding the remaining
    /// TM-SDU bits, and the [`FragEndKind`] the remainder must be completed
    /// with. Returns `None` if the SDU is not actually larger than one fragment
    /// (caller should use the self-contained [`Self::build_mac_access_block`]
    /// path instead), or if the remainder is larger than
    /// [`mac::wire::MAX_UL_FRAG_SLOTS`] full slots can carry (no realistic signalling SDU
    /// is that large).
    pub fn build_mac_access_frag_start(issi: u32, sdu: &mut BitBuffer) -> Option<(BitBuffer, BitBuffer, FragEndKind)> {
        let bits = pack::<512>(sdu).ok()?;
        let (block, remainder, kind) = mac::wire::access_fragment(issi, bits.view()).ok()?;
        sdu.seek(sdu.get_len());
        Some((unpack(&block), unpack(&remainder), kind))
    }

    /// Build the MAC-END-HU block that completes a fragmented uplink transfer
    /// (ETSI TS 100 392-2 cl. 21.4.2.2, cl. 23.4.2.1.2). It carries the
    /// `remainder` of the TM-SDU in a single granted SCH/HU subslot, terminated
    /// by a **length indication** (octet count) so the receiver reassembles
    /// exactly the original TM-SDU. Fill bits pad the PDU to a byte boundary
    /// (cl. 23.4.2.2): a bit "1" immediately after the TM-SDU followed by bits
    /// "0" to the next octet boundary. The remaining bits of the 92-bit block
    /// (beyond the length indication) are left zero and are ignored by the
    /// receiver (it reads only `length_ind * 8` bits).
    ///
    /// Returns the full 92-bit type-1 block, or `None` if the remainder is too
    /// large for one MAC-END-HU (the caller then uses the full-slot MAC-END-UL
    /// path instead).
    pub fn build_mac_end_hu_block(remainder: &mut BitBuffer) -> Option<BitBuffer> {
        let bits = pack::<512>(remainder).ok()?;
        let block = mac::wire::end_hu(bits.view()).ok()?;
        remainder.seek(remainder.get_len());
        Some(unpack(&block))
    }

    /// Build the MAC-END-UL block that completes a fragmented uplink transfer on
    /// a granted SCH/F full slot (ETSI TS 100 392-2 cl. 21.4.2.5,
    /// cl. 23.4.2.1.2). It carries the `remainder` of the TM-SDU in the 268-bit
    /// SCH/F block, terminated by a **length indication** (octet count) so the
    /// receiver reassembles exactly the original TM-SDU. Fill bits pad the PDU
    /// to a byte boundary (cl. 23.4.2.2). The remaining bits of the 268-bit
    /// block (beyond the length indication) are left zero and are ignored by the
    /// receiver (it reads only `length_ind * 8` bits).
    ///
    /// Returns the full 268-bit type-1 block, or `None` if the remainder is too
    /// large for one MAC-END-UL (which would require multi-slot fragmentation
    /// with MAC-FRAG-UL continuations — not implemented).
    pub fn build_mac_end_ul_block(remainder: &mut BitBuffer) -> Option<BitBuffer> {
        let bits = pack::<512>(remainder).ok()?;
        let block = mac::wire::end_ul(bits.view()).ok()?;
        remainder.seek(remainder.get_len());
        Some(unpack(&block))
    }

    /// Take the MAC block queued for uplink transmission, if any. Consumed by
    /// the uplink PHY driver once a valid access opportunity is reached
    /// (Phase 3d).
    pub fn take_pending_uplink(&mut self) -> Option<PendingUplink> {
        self.pending_uplink.take().map(|pending| PendingUplink {
            mac_block: unpack(&pending.mac_block),
            logical_channel: desktop_channel(pending.logical_channel),
            scrambling_code: pending.scrambling_code,
            tx_reporter: pending.tx_reporter.and_then(|handle| self.tx_reporters.remove(&handle)),
            pdu_priority: pending.pdu_priority,
            is_emergency: pending.is_emergency,
        })
    }

    /// The MS's own Individual Short Subscriber Identity (ISSI) from config.
    #[cfg(test)]
    fn own_issi(&self) -> u32 {
        self.config
            .config()
            .ms
            .as_ref()
            .expect("MS config section required in MS mode")
            .issi
    }

    /// MS receive filter (ETSI TS 100 392-2 clause 23 addressing): decide
    /// whether a downlink MAC PDU carrying `addr` is destined for this mobile
    /// station.
    ///
    /// At the MAC layer an individual (ISSI) and a group (GSSI) identity are
    /// both carried as a bare 24-bit SSI — `MacResourceAddrType` has no
    /// distinct group code — so ownership is decided purely by SSI value: the
    /// MS accepts its own ISSI, any group it has attached to, and the
    /// broadcast address, and ignores traffic addressed to other
    /// subscribers/groups.
    ///
    /// Filtering applies ONLY in MS mode. In monitor mode (and any non-MS
    /// mode) the receiver is passive and must observe all traffic, so nothing
    /// is filtered.
    fn accept_downlink_address(&self, addr: &TetraAddress) -> bool {
        let cfg = self.config.config();
        self.mac.accepts(cfg.stack_mode == StackMode::Ms && cfg.ms.is_some(), *addr)
    }

    fn rx_tlmb_prim(&mut self, _queue: &mut MessageQueue, _message: SapMsg) {
        tracing::trace!("rx_tlmb_prim");
        unimplemented!();
    }

    fn update_scrambing_and_submit_to_lmac(&mut self, queue: &mut MessageQueue) {
        if self.mac.update_scrambling().is_some() {
            tracing::trace!(
                "compute_scrambling_and_submit_to_lmac cc {} mcc {} mnc {} scrambling_code: {}",
                self.cc.unwrap(),
                self.mcc.unwrap(),
                self.mnc.unwrap(),
                self.scrambling_code.unwrap()
            );

            let m = SapMsg {
                sap: Sap::TmvSap,
                src: self.self_component,
                dest: TetraEntity::Lmac,
                msg: SapMsgInner::TmvConfigureReq(TmvConfigureReq {
                    scrambling_code: self.scrambling_code,
                    ..Default::default()
                }),
            };
            queue.push_back(m);
        }
    }

    fn rx_tlmc_configure_req(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        tracing::trace!("rx_tlmc_configure_req");
        let SapMsgInner::TlmcConfigureReq(prim) = &message.msg else {
            panic!()
        };

        if let Some(valid_addresses) = &prim.valid_addresses {
            tracing::debug!("rx_tlmc_configure_req: valid_addresses: {:?}", valid_addresses);

            // Update the runtime downlink address filter when the MLE supplies
            // the MS's identities (cl. 17.3.2 / 23.4.1.2.1). `None` members mean
            // "leave unchanged" (e.g. a scrambling-only configure at cell
            // selection), so the filter is only touched by an MLE-IDENTITIES
            // request carrying the attached set.
            if let Err(error) = self.mac.configure_identities(
                valid_addresses.mcc,
                valid_addresses.mnc,
                valid_addresses.individual_ssi,
                valid_addresses.group_ssis.as_deref(),
            ) {
                tracing::warn!("MS identities exceed portable MAC capacity: {:?}", error);
                return;
            }

            // Attempt to update scrambling code (if cc is also known)
            self.update_scrambing_and_submit_to_lmac(queue);
        } else {
            tracing::warn!("rx_tlmc_configure_req: No valid addresses provided");
        }
    }

    fn rx_tlmc_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        tracing::trace!("rx_tlmc_prim");
        match message.msg {
            SapMsgInner::TlmcConfigureReq(_) => {
                self.rx_tlmc_configure_req(queue, message);
            }
            SapMsgInner::TlmcTuneReq(_) => {
                self.rx_tlmc_tune_req(queue, message);
            }
            SapMsgInner::TlmcUPlaneConfigureReq(prim) => {
                // MLE-CONFIGURE U-plane transmit state (cl. 17.3.3 / 14.5.1.4),
                // forwarded down from CC-MS via MLE. This MS may emit uplink
                // TCH/S traffic only while it holds the transmission grant AND
                // the U-plane is switched on (it is the current talker). The
                // granted slot is not carried here — it stays owned by the
                // CHANNEL ALLOCATION record (cl. 21.5.2). On de-grant, flush the
                // uplink jitter buffer so a later grant cannot emit stale audio.
                let granted = prim.tx_grant && prim.switch_u_plane;
                if granted != self.uplink_tx_granted {
                    tracing::info!(
                        "UMAC-MS: U-plane transmit grant {} (tx_grant={}, switch_u_plane={})",
                        if granted { "acquired" } else { "released" },
                        prim.tx_grant,
                        prim.switch_u_plane
                    );
                }
                // Releasing the U-plane (switch off) frees the single traffic
                // resource, so the individual-call engagement lock is cleared and
                // a subsequently-notified group call may take the channel
                // (cl. 14.2.4.1 / 14.5.1.1). While an individual call is engaged
                // the CMCE gate keeps a group's switch-off from reaching here, so
                // this only fires on the engaged call's own release.
                self.mac.configure_uplane(prim.switch_u_plane, prim.tx_grant);
            }
            _ => {
                panic!();
            }
        }
    }

    /// MS runtime downlink retune (**[impl policy]**): forward the MLE's tune
    /// request down to the lower MAC (TLMC -> TMV). UMAC holds no radio state for
    /// this; it is pure pass-through so the PHY ultimately retunes the SDR.
    fn rx_tlmc_tune_req(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        let SapMsgInner::TlmcTuneReq(prim) = &message.msg else { panic!() };
        let carrier_hz = prim.carrier_hz;
        tracing::info!("UMAC: forwarding MS downlink retune to {} Hz (TLMC -> TMV)", carrier_hz);
        queue.push_back(SapMsg {
            sap: Sap::TmvSap,
            src: TetraEntity::Umac,
            dest: TetraEntity::Lmac,
            msg: SapMsgInner::TmvTuneReq(TmvTuneReq { carrier_hz }),
        });
    }

    /// Act on a CHANNEL ALLOCATION element (cl. 21.5.2) received in a
    /// MAC-RESOURCE addressed to this MS, recording which downlink timeslot(s)
    /// carry our assigned traffic channel so the U-plane TMD relay follows the
    /// assigned slot (cl. 14.5.1.3). A same-carrier TCH is on some slot other
    /// than the control channel, so the slot is taken from the element and never
    /// hardcoded.
    ///
    /// M2 handles same-carrier allocations only. A cross-carrier or cell-change
    /// allocation (different `carrier_num`, `cell_change_flag`, or extended
    /// carrier numbering) requires a downlink PHY retune, which is deferred to
    /// M3 — here it is logged and ignored (no retune, no slot change), so the MS
    /// keeps decoding its current channel rather than acting half-way on an
    /// allocation it cannot yet follow.
    #[cfg(test)]
    fn act_on_channel_allocation(&mut self, ca: &ChanAllocElement, alloc_ssi: u32) {
        let own = self.own_issi();
        let result = self.mac.allocate(
            own,
            Allocation {
                carrier: ca.carrier_num,
                extended: ca.ext.is_some(),
                cell_change: ca.cell_change_flag,
                downlink: matches!(ca.ul_dl_assigned, UlDlAssignment::Dl | UlDlAssignment::Both),
                kind: match ca.alloc_type {
                    ChanAllocType::Additional => AllocationType::Additional,
                    _ => AllocationType::Replace,
                },
                slots: ca.ts_assigned,
                owner: alloc_ssi,
            },
        );
        tracing::info!(
            "rx_mac_resource: CHANNEL ALLOCATION {} on carrier {} for SSI {}: {:?}, slots {:?}",
            ca.alloc_type,
            ca.carrier_num,
            alloc_ssi,
            result,
            self.assigned_traffic_slots
        );
    }

    /// Relay decoded downlink circuit-mode (TCH/S) speech up to CMCE.
    ///
    /// The LMAC decodes each downlink traffic burst (TCH/S) and delivers it over
    /// the TMD-SAP (cl. 23) tagged with the timeslot it arrived on. On the MS the
    /// U-plane switch and call state live in CMCE (CC-MS, cl. 14.5.1.4), so the
    /// MAC simply forwards the frame upward; it performs no audio processing.
    ///
    /// As a MAC-level guard it relays only traffic whose slot carried an AACH
    /// downlink usage marker (cl. 21.4.7.2) that the serving cell assigned to a
    /// call this MS participates in (cl. 23.5.5, recorded in
    /// `active_traffic_usage_markers`). The usage marker — not a static timeslot
    /// bitmap — is the authoritative traffic-channel identity: it identifies the
    /// call regardless of which physical slot the cell places it on, and it is
    /// exactly what the AACH and the STCH talker-identity path already follow.
    /// Traffic for markers we do not participate in (other groups' calls) is
    /// dropped here. Each relayed frame is tagged with the marker and the owning
    /// SSI so CC-MS can demultiplex concurrent calls; the definitive U-plane
    /// switch gate (is the call actually receiving) remains in CC-MS.
    fn rx_tmd_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        tracing::trace!("rx_tmd_prim");
        match message.msg {
            SapMsgInner::TmdCircuitDataInd(mut prim) => {
                // Gate on the AACH usage marker of the slot this block arrived
                // on (cl. 21.4.7.2 / 23.5.5): forward only when the marker is one
                // the serving cell assigned to a call this MS participates in.
                let Some((marker, owner_ssi)) = self.mac.downlink_traffic_owner() else {
                    tracing::trace!(
                        "rx_tmd_prim: usage marker {:?} on ts={} not assigned to any of our calls, dropping speech frame",
                        self.current_traffic_usage_marker,
                        prim.ts
                    );
                    return;
                };
                prim.usage_marker = Some(marker);
                prim.owner_ssi = Some(owner_ssi);
                queue.push_back(SapMsg {
                    sap: Sap::TmdSap,
                    src: TetraEntity::Umac,
                    dest: TetraEntity::Cmce,
                    msg: SapMsgInner::TmdCircuitDataInd(prim),
                });
            }
            SapMsgInner::TmdCircuitDataReq(prim) => {
                // Uplink U-plane source frame from CC-MS (the U-plane owner, cl.
                // 14.5.1.4). UMAC is the transmit timing authority (cl. 23): it
                // buffers the frame here and clocks exactly one out per granted
                // uplink traffic slot in `tick_start`. The `ts` field is not used
                // for slot selection — the assigned uplink slot comes from the
                // CHANNEL ALLOCATION record (cl. 21.5.2). Drop-oldest keeps the
                // buffer bounded if CC supplies faster than we emit.
                let len = (prim.data.len().min(35) * 8).min(274) as u16;
                let frame = tetra_ms_core::types::BitView::new(&prim.data, len).expect("audio length checked");
                self.mac.queue_audio(frame).expect("audio capacity checked");
            }
            _ => {
                panic!("UMAC-MS: unexpected message on TMD-SAP: {:?}", message.msg);
            }
        }
    }
}

impl TetraEntityTrait for UmacMs {
    fn entity(&self) -> TetraEntity {
        TetraEntity::Umac
    }

    fn set_config(&mut self, config: SharedConfig) {
        let (role, issi) = {
            let config = config.config();
            (config.stack_mode, config.ms.as_ref().map(|ms| ms.issi))
        };
        let changed = role != self.configured_role || issi != self.configured_issi;
        self.config = config;
        if changed && let Err(error) = self.reset() {
            self.clear_protocol();
            self.configured_role = role;
            self.configured_issi = issi;
            tracing::warn!(
                ?error,
                "UMAC role/identity changed with an invalid address filter; pending transfers cancelled"
            );
        }
    }

    fn rx_prim(&mut self, queue: &mut MessageQueue, message: SapMsg) {
        self.synchronize_role();
        tracing::trace!("rx_prim: {:?}", message);
        // tracing::debug!(ts=%message.dltime, "rx_prim: {:?}", message);

        match message.sap {
            Sap::TmvSap => {
                self.rx_tmv_prim(queue, message);
            }

            Sap::TmaSap => {
                self.rx_tma_prim(queue, message);
            }

            Sap::TlmbSap => {
                self.rx_tlmb_prim(queue, message);
            }

            Sap::TlmcSap => {
                self.rx_tlmc_prim(queue, message);
            }

            // U-plane: decoded downlink circuit-mode (TCH/S) speech from the LMAC
            // (cl. 23, TMD-SAP). The MS MAC relays it up to CMCE, which owns the
            // U-plane switch; the MAC itself performs no audio processing.
            Sap::TmdSap => {
                self.rx_tmd_prim(queue, message);
            }

            _ => {
                panic!()
            }
        }
    }

    fn tick_start(&mut self, queue: &mut MessageQueue, ts: TdmaTime) {
        self.synchronize_role();
        // Advance the absolute downlink time to stay phase-locked to the slot
        // the PHY just demodulated. In MS mode the stack is receive-timed: the
        // router blocks on the downlink and drives `tick_start` with the
        // demodulator-recovered slot time (`messagerouter::run_stack_ms`), which
        // is the true physical-slot clock the modulator's TX timing is anchored
        // to (ETSI TS 100 392-2 cl. 9.5.1: the MS synchronises transmit timing
        // to the received downlink). A single `drive_rx` cycle can yield more
        // than one demodulated slot (cl. 9.3, the single RX channel walks the
        // frame), so `ts` may advance by several slots per tick; a blind `+1`
        // would then desync UMAC's slot phase from the physically-transmitted
        // slot and scatter uplink traffic bursts off the BS-assigned timeslot.
        // Advance by the *actual* recovered delta instead, preserving the
        // network-absolute frame/multiframe/hyperframe numbering re-seeded from
        // each BSCH SYNC in `rx_tmv_bsch` (cl. 7 / 21.4.4.2). Before the first
        // recovered slot there is no reference, so bootstrap with `+1` (pre-sync
        // `dltime` is provisional until the first SYNC re-seed anyway).
        let actions = match self.mac.tick(ts) {
            Ok(actions) => actions,
            Err(error) => {
                tracing::warn!("UMAC invalid recovered time: {:?}", error);
                return;
            }
        };

        // Drive any in-flight multi-slot reserved uplink transfer: emit the next
        // MAC-FRAG-UL/MAC-END-UL block when its reserved slot (dltime + 2) is
        // reached (cl. 23.4.2.1.2 / 23.5.2). No-op when nothing is reserved.
        self.apply_reserved_action(queue, actions.reserved);

        // Emit the MS uplink TCH/S traffic burst on the granted slot when this
        // MS is the current talker (cl. 23 transmit scheduling / cl. 14.5.1.4
        // U-plane). No-op unless a U-plane transmit grant and an assigned
        // traffic slot are both active.
        if let Some(burst) = actions.traffic {
            self.emit_core_traffic(queue, burst);
        }
    }
}

#[cfg(test)]
#[path = "umac_ms_wire_tests.rs"]
mod wire_tests;

#[cfg(test)]
#[path = "umac_ms_receipt_tests.rs"]
mod receipt_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use tetra_config::magentastation::from_toml_str;
    use tetra_core::SsiType;
    use tetra_ms_core::types::FragmentBits;
    use tetra_pdus::umac::enums::access_assign_dl_usage::AccessAssignDlUsage;
    use tetra_pdus::umac::enums::access_assign_ul_usage::AccessAssignUlUsage;
    use tetra_pdus::umac::fields::sysinfo_default_def_for_access_code_a::SysinfoDefaultDefForAccessCodeA;
    use tetra_pdus::umac::pdus::access_assign::AccessField;
    use tetra_pdus::umac::pdus::{mac_end_ul::MacEndUl, mac_frag_ul::MacFragUl};
    use tetra_saps::tlmc::{TlmcConfigureReq, TlmcValidAddress};
    use tetra_saps::tma::TmaUnitdataReq;

    /// Minimal valid MS config (mirrors `example_config/config-ms.toml`); the
    /// DL/UL frequencies must match what `cell_info` recomputes.
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

    pub(super) fn ms_umac() -> UmacMs {
        let cfg = from_toml_str(MS_TOML).expect("valid MS test config");
        UmacMs::new(SharedConfig::from_parts(cfg, None))
    }

    /// `tick_start` must advance the absolute downlink time by the *actual*
    /// number of slots the demodulator recovered since the previous tick, not a
    /// blind `+1`. In MS mode the stack is receive-timed and a single `drive_rx`
    /// cycle can yield more than one slot (cl. 9.3), so the router hands
    /// `tick_start` a recovered downlink time that may jump several slots; UMAC's
    /// slot phase must track it exactly or uplink bursts scatter off the
    /// BS-assigned timeslot (ETSI TS 100 392-2 cl. 9.5.1). A BSCH SYNC re-seed of
    /// `dltime` (done in `rx_tmv_bsch`) stays authoritative: subsequent ticks
    /// advance from the re-seeded value by the recovered delta.
    #[test]
    fn test_tick_start_advances_by_recovered_delta() {
        let mut umac = ms_umac();
        let start = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        umac.dltime = start;
        let mut q = MessageQueue::new();

        // First tick has no previous reference: bootstrap by one slot and record
        // the recovered time.
        let rx0 = TdmaTime { t: 3, f: 5, m: 10, h: 0 };
        umac.tick_start(&mut q, rx0);
        assert_eq!(umac.dltime, start.add_timeslots(1), "bootstrap advances one slot");
        assert_eq!(umac.last_recovered_dltime, Some(rx0));

        // Next demodulated batch jumped two slots: dltime must advance by two,
        // not one.
        let rx1 = rx0.add_timeslots(2);
        umac.tick_start(&mut q, rx1);
        assert_eq!(umac.dltime, start.add_timeslots(3), "advances by the recovered 2-slot delta");
        assert_eq!(umac.last_recovered_dltime, Some(rx1));

        // A BSCH SYNC re-seed re-anchors dltime to the network-absolute time
        // (as rx_tmv_bsch does); the following tick advances from the re-seeded
        // base by the next recovered delta.
        let reseed = TdmaTime { t: 2, f: 18, m: 44, h: 3 };
        umac.dltime = reseed;
        let rx2 = rx1.add_timeslots(1);
        umac.tick_start(&mut q, rx2);
        assert_eq!(
            umac.dltime,
            reseed.add_timeslots(1),
            "advances from the SYNC-re-seeded base by the recovered delta"
        );
    }

    /// An ACCESS-ASSIGN granting an ongoing-frame random access opportunity for
    /// access code A on both uplink subslots (single access field, header == 1;
    /// base frame length 0b0010 = ongoing frame, cl. 21.4.7.2 / Table 21.85).
    pub(super) fn ongoing_a_assign() -> AccessAssign {
        AccessAssign {
            _header: 1,
            dl_usage: AccessAssignDlUsage::CommonControl,
            ul_usage: AccessAssignUlUsage::CommonOnly,
            f1_af1: None,
            f2_af2: None,
            f2_af: Some(AccessField {
                access_code: 0, // code A
                base_frame_len: 0b0010,
            }),
        }
    }

    /// An "assigned only" ACCESS-ASSIGN: an MS on its common control channel
    /// treats this as a reserved slot, i.e. no random access opportunity
    /// (cl. 23.5.1.4.2).
    fn reserved_assign() -> AccessAssign {
        AccessAssign {
            _header: 1,
            dl_usage: AccessAssignDlUsage::CommonControl,
            ul_usage: AccessAssignUlUsage::AssignedOnly,
            f1_af1: None,
            f2_af2: None,
            f2_af: None,
        }
    }

    /// Set up a UMAC with a queued uplink block and an active random access
    /// attempt (IMM == 15 = immediate access), as after `rx_tma_prim`.
    fn umac_with_active_attempt() -> UmacMs {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        umac.scrambling_code = Some(0x1234_5678);

        umac.access_params.update_sysinfo_default_a(&SysinfoDefaultDefForAccessCodeA {
            imm: 15,
            wt: 6,
            nu: 4,
            fl_factor: false,
            ts_ptr: 0,
            min_pdu_prio: 0,
        });

        let mut sdu = BitBuffer::from_bitstr("0110100100011110001011010010");
        let mac_block = UmacMs::build_mac_access_block(umac.own_issi(), &mut sdu).expect("SDU fits");
        umac.pending_uplink = Some(CorePendingUplink {
            mac_block: pack(&mac_block).unwrap(),
            logical_channel: Channel::SchHu,
            scrambling_code: 0x1234_5678,
            tx_reporter: None,
            pdu_priority: None,
            is_emergency: false,
        });

        let p = umac.access_params.params_for(AccessCode::A).expect("params present").clone();
        umac.random_access
            .initiate(umac.mac.dltime, AccessCode::A, &p, 0, false)
            .expect("initiate succeeds");
        assert!(umac.random_access.is_active());
        umac
    }

    /// Set up a UMAC camped with access params (advertising a minimum PDU
    /// priority) and a queued uplink of the given L3 priority/emergency, but with
    /// random access NOT yet initiated — so `drive_random_access` runs the
    /// cl. 23.5.1.4.4 permission gate against the queued priority.
    pub(super) fn umac_with_queued_priority(min_pdu_prio: u8, pdu_priority: Option<u8>, is_emergency: bool) -> UmacMs {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        umac.scrambling_code = Some(0x1234_5678);
        umac.access_params.update_sysinfo_default_a(&SysinfoDefaultDefForAccessCodeA {
            imm: 15,
            wt: 6,
            nu: 4,
            fl_factor: false,
            ts_ptr: 0,
            min_pdu_prio,
        });
        let mut sdu = BitBuffer::from_bitstr("0110100100011110001011010010");
        let mac_block = UmacMs::build_mac_access_block(umac.own_issi(), &mut sdu).expect("SDU fits");
        umac.pending_uplink = Some(CorePendingUplink {
            mac_block: pack(&mac_block).unwrap(),
            logical_channel: Channel::SchHu,
            scrambling_code: 0x1234_5678,
            tx_reporter: None,
            pdu_priority,
            is_emergency,
        });
        umac
    }

    /// Feature 2: `drive_random_access` feeds the queued L3 PDU priority to the
    /// permission gate (cl. 23.5.1.4.4). A non-emergency uplink whose priority is
    /// below the access code's advertised minimum is not permitted, so the
    /// attempt is not started and the block is dropped (the LLC retransmits).
    #[test]
    fn test_below_min_priority_uplink_rejected() {
        let mut umac = umac_with_queued_priority(5, Some(3), false);
        let mut q = MessageQueue::new();

        umac.drive_random_access(&mut q, &ongoing_a_assign());

        assert!(!umac.random_access.is_active(), "gate must reject below-min priority");
        assert!(umac.pending_uplink.is_none(), "rejected uplink is dropped");
        assert!(q.pop_front().is_none(), "no uplink emitted");
    }

    /// Feature 2: an emergency uplink on access code A bypasses the priority gate
    /// (cl. 23.5.1.4.4), so even a below-minimum priority starts the attempt.
    #[test]
    fn test_emergency_uplink_bypasses_priority_gate() {
        let mut umac = umac_with_queued_priority(5, Some(3), true);
        let mut q = MessageQueue::new();

        umac.drive_random_access(&mut q, &ongoing_a_assign());

        assert!(umac.random_access.is_active(), "emergency access on code A bypasses the gate");
        assert!(umac.pending_uplink.is_some(), "block retained for transmission");
    }

    /// 3C-d: on a granting ACCESS-ASSIGN, the UMAC emits the queued MAC-ACCESS
    /// block to LMAC as a TMV-UNITDATA request on SCH/HU at DL+2 timeslots.
    #[test]
    fn test_random_access_emits_uplink_on_opportunity() {
        let mut umac = umac_with_active_attempt();
        let mut q = MessageQueue::new();

        umac.drive_random_access(&mut q, &ongoing_a_assign());

        let msg = q.pop_front().expect("an uplink block should be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        assert_eq!(msg.dest, TetraEntity::Lmac);

        let SapMsgInner::TmvUnitdataReq(slot) = msg.msg else {
            panic!("expected TmvUnitdataReq");
        };
        assert_eq!(slot.ts, umac.dltime.add_timeslots(2), "uplink is DL + 2 timeslots");
        let blk = slot.blk1.expect("blk1 carries the MAC-ACCESS");
        assert_eq!(blk.logical_channel, LogicalChannel::SchHu);
        assert_eq!(blk.scrambling_code, 0x1234_5678);
        assert_eq!(blk.mac_block.get_len(), SCH_HU_TYPE1_BITS, "92-bit SCH/HU type-1 block");
    }

    #[test]
    fn random_access_selected_subslot_reaches_tmv() {
        for (subslot, placement) in [
            (Subslot::One, TxBurstPlacement::Subslot1),
            (Subslot::Two, TxBurstPlacement::Subslot2),
        ] {
            let mut umac = umac_with_active_attempt();
            let mut queue = MessageQueue::new();
            let time = umac.dltime.add_timeslots(2);
            umac.emit_uplink(&mut queue, time, subslot);
            let SapMsgInner::TmvUnitdataReq(slot) = queue.pop_front().unwrap().msg else {
                panic!("expected TMV uplink");
            };
            assert_eq!(slot.placement, placement);
            assert_eq!(slot.ts, time);
            assert!(!slot.reserved_access);
            assert_eq!(slot.blk1.unwrap().logical_channel, LogicalChannel::SchHu);
        }
    }

    /// 3C-d: a reserved (assigned-only) slot is not a random access opportunity,
    /// so nothing is transmitted and the attempt stays active (cl. 23.5.1.4.2).
    #[test]
    fn test_reserved_slot_emits_nothing() {
        let mut umac = umac_with_active_attempt();
        let mut q = MessageQueue::new();

        umac.drive_random_access(&mut q, &reserved_assign());

        assert!(q.pop_front().is_none(), "no uplink on a reserved slot");
        assert!(umac.random_access.is_active(), "attempt still pending");
        assert!(umac.pending_uplink.is_some(), "queued block retained");
    }

    /// 3C-d: with no active attempt, a granting ACCESS-ASSIGN is ignored (the
    /// state machine must have been initiated by an uplink request first).
    #[test]
    fn test_no_attempt_ignores_opportunity() {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        let mut q = MessageQueue::new();

        umac.drive_random_access(&mut q, &ongoing_a_assign());

        assert!(q.pop_front().is_none(), "idle state machine emits nothing");
    }

    // --- Uplink fragmentation grant handling (cl. 23.5.2 / 23.4.2.1.2) ---

    /// Once the BS acknowledges the frag-start with a MAC-RESOURCE granting one
    /// subslot, the UMAC transmits the buffered remainder as a MAC-END-HU on
    /// SCH/HU (DL + 2 timeslots) and clears the pending fragment.
    #[test]
    fn test_grant_emits_mac_end_hu() {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        let reporter = TxReporter::new();
        umac.pending_fragment = Some(UplinkFragment {
            remainder: pack(&BitBuffer::from_bitstr(&"1".repeat(40))).unwrap(),
            scrambling_code: 0xABCD_1234,
            end_kind: FragEndKind::MacEndHu,
            tx_reporter: umac.track_reporter(Some(reporter.clone())).unwrap(),
        });
        let mut q = MessageQueue::new();

        let grant = BasicSlotgrant {
            capacity_allocation: BasicSlotgrantCapAlloc::FirstSubslotGranted,
            granting_delay: BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
        };
        umac.emit_fragment_end(&mut q, Some(&grant));

        let msg = q.pop_front().expect("MAC-END-HU should be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        assert_eq!(msg.dest, TetraEntity::Lmac);
        let SapMsgInner::TmvUnitdataReq(slot) = msg.msg else {
            panic!("expected TmvUnitdataReq");
        };
        assert_eq!(slot.ts, umac.dltime.add_timeslots(2), "MAC-END-HU at DL + 2 timeslots");
        assert!(slot.reserved_access, "MAC-END-HU is a reserved-access burst");
        assert_eq!(slot.placement, TxBurstPlacement::Subslot1);
        let blk = slot.blk1.expect("blk1 carries the MAC-END-HU");
        assert_eq!(blk.logical_channel, LogicalChannel::SchHu);
        assert_eq!(blk.scrambling_code, 0xABCD_1234);
        assert_eq!(blk.mac_block.get_len(), SCH_HU_TYPE1_BITS, "92-bit SCH/HU type-1 block");
        assert!(umac.pending_fragment.is_none(), "fragment consumed");
        // The LLC transmit receipt is marked transmitted so the acknowledged
        // basic link starts its ack-wait/retransmit timer (cl. 22.3.2.3).
        assert!(reporter.is_transmitted(), "MAC-END-HU emission marks the receipt transmitted");
    }

    /// A remainder too large for a MAC-END-HU subslot is completed by a
    /// MAC-END-UL on a granted SCH/F full slot (Normal Uplink Burst): once the
    /// BS acknowledges the frag-start granting one full slot (`Grant1Slot`), the
    /// UMAC transmits the remainder as a 268-bit MAC-END-UL on SCH/F at DL + 2
    /// timeslots as a reserved-access burst (cl. 23.4.2.1.2 / 23.5.2).
    #[test]
    fn test_grant_emits_mac_end_ul_full_slot() {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        let reporter = TxReporter::new();
        // 160-bit remainder: too large for a MAC-END-HU (max ~81 bits), fits a
        // MAC-END-UL full slot (max ~254 bits).
        umac.pending_fragment = Some(UplinkFragment {
            remainder: pack(&BitBuffer::from_bitstr(&"1".repeat(160))).unwrap(),
            scrambling_code: 0x0BAD_F00D,
            end_kind: FragEndKind::MacEndUl,
            tx_reporter: umac.track_reporter(Some(reporter.clone())).unwrap(),
        });
        let mut q = MessageQueue::new();

        let grant = BasicSlotgrant {
            capacity_allocation: BasicSlotgrantCapAlloc::Grant1Slot,
            granting_delay: BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
        };
        umac.emit_fragment_end(&mut q, Some(&grant));

        let msg = q.pop_front().expect("MAC-END-UL should be emitted");
        assert!(q.pop_front().is_none(), "exactly one message emitted");
        assert_eq!(msg.dest, TetraEntity::Lmac);
        let SapMsgInner::TmvUnitdataReq(slot) = msg.msg else {
            panic!("expected TmvUnitdataReq");
        };
        assert_eq!(slot.ts, umac.dltime.add_timeslots(2), "MAC-END-UL at DL + 2 timeslots");
        assert!(slot.reserved_access, "MAC-END-UL is a reserved-access burst");
        let blk = slot.blk1.expect("blk1 carries the MAC-END-UL");
        assert_eq!(blk.logical_channel, LogicalChannel::SchF, "full-slot end uses SCH/F");
        assert_eq!(blk.scrambling_code, 0x0BAD_F00D);
        assert_eq!(blk.mac_block.get_len(), SCH_F_TYPE1_BITS, "268-bit SCH/F type-1 block");
        assert!(umac.pending_fragment.is_none(), "fragment consumed");
        assert!(reporter.is_transmitted(), "MAC-END-UL emission marks the receipt transmitted");
    }

    /// A grant whose capacity does not match the requested fragment-end kind
    /// (here a subslot grant when a full slot was requested) cannot complete the
    /// transfer: nothing is transmitted, the fragment is dropped and its receipt
    /// marked discarded so the LLC retransmits (cl. 23.5.2 / 22.3.2.3).
    #[test]
    fn test_mismatched_grant_drops_fragment() {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        let reporter = TxReporter::new();
        umac.pending_fragment = Some(UplinkFragment {
            remainder: pack(&BitBuffer::from_bitstr(&"1".repeat(160))).unwrap(),
            scrambling_code: 0,
            end_kind: FragEndKind::MacEndUl,
            tx_reporter: umac.track_reporter(Some(reporter.clone())).unwrap(),
        });
        let mut q = MessageQueue::new();

        let grant = BasicSlotgrant {
            capacity_allocation: BasicSlotgrantCapAlloc::FirstSubslotGranted,
            granting_delay: BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
        };
        umac.emit_fragment_end(&mut q, Some(&grant));

        assert!(q.pop_front().is_none(), "no uplink on a mismatched grant");
        assert!(umac.pending_fragment.is_none(), "fragment consumed even on mismatch");
        assert!(reporter.is_discarded(), "mismatched-grant drop marks the receipt discarded");
    }

    /// A MAC-RESOURCE that carries no slot grant cannot complete the fragmented
    /// transfer: nothing is transmitted and the fragment is dropped. The LLC
    /// transmit receipt is marked discarded so the acknowledged basic link
    /// retransmits the whole transfer (cl. 22.3.2.3).
    #[test]
    fn test_no_grant_drops_fragment() {
        let mut umac = ms_umac();
        let reporter = TxReporter::new();
        umac.pending_fragment = Some(UplinkFragment {
            remainder: pack(&BitBuffer::from_bitstr(&"1".repeat(40))).unwrap(),
            scrambling_code: 0,
            end_kind: FragEndKind::MacEndHu,
            tx_reporter: umac.track_reporter(Some(reporter.clone())).unwrap(),
        });
        let mut q = MessageQueue::new();

        umac.emit_fragment_end(&mut q, None);

        assert!(q.pop_front().is_none(), "no uplink without a grant");
        assert!(umac.pending_fragment.is_none(), "fragment consumed even without a grant");
        assert!(reporter.is_discarded(), "no-grant drop marks the receipt discarded");
    }

    // --- Multi-slot uplink fragmentation (cl. 23.4.2.1.2 / 23.5.2) ---

    /// A remainder too large for one full slot is split into MAC-FRAG-UL
    /// continuations plus a terminating MAC-END-UL across the N reserved full
    /// slots the BS granted. Emission is discontinuous: exactly one block is put
    /// on air per reserved slot (the PHY can only transmit at `dltime + 2`), so
    /// the first block goes out when the grant is processed and the rest as each
    /// reserved slot is reached on later downlink ticks.
    #[test]
    fn test_multislot_grant_schedules_and_drives() {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        let reporter = TxReporter::new();
        // 400-bit remainder => 1 MAC-FRAG-UL (264 SDU bits) + MAC-END-UL (136),
        // i.e. total_slots = 2.
        umac.pending_fragment = Some(UplinkFragment {
            remainder: pack(&BitBuffer::from_bitstr(&"1".repeat(400))).unwrap(),
            scrambling_code: 0xFEED_BEEF,
            end_kind: FragEndKind::MacFragUl { total_slots: 2 },
            tx_reporter: umac.track_reporter(Some(reporter.clone())).unwrap(),
        });
        let mut q = MessageQueue::new();

        let grant = BasicSlotgrant {
            capacity_allocation: BasicSlotgrantCapAlloc::Grant2Slots,
            granting_delay: BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
        };
        let slot0 = umac.dltime.add_timeslots(2);
        umac.emit_fragment_end(&mut q, Some(&grant));

        // First reserved slot (dltime + 2) is due this tick: exactly one
        // MAC-FRAG-UL emitted; the plan retains the MAC-END-UL.
        let msg = q.pop_front().expect("first MAC-FRAG-UL emitted");
        assert!(q.pop_front().is_none(), "only one block per slot");
        let SapMsgInner::TmvUnitdataReq(slot) = msg.msg else {
            panic!("expected TmvUnitdataReq");
        };
        assert_eq!(slot.ts, slot0, "first block at DL + 2 timeslots");
        assert!(slot.reserved_access, "reserved-access burst");
        let mut blk0 = slot.blk1.expect("blk1").mac_block;
        assert_eq!(blk0.get_len(), SCH_F_TYPE1_BITS, "full-slot SCH/F block");
        let frag_pdu = MacFragUl::from_bitbuf(&mut blk0).expect("MAC-FRAG-UL decodes");
        assert!(!frag_pdu.fill_bits, "continuation fills the slot, no fill bits");
        assert!(umac.pending_fragment.is_none(), "fragment consumed");
        assert!(umac.reserved_tx.is_some(), "plan still holds the MAC-END-UL");
        assert!(!reporter.is_transmitted(), "receipt not marked until the last block");

        // Advance downlink ticks until the second reserved slot (slot0 + 4) is
        // reached; the MAC-END-UL is emitted then and the receipt marked. The
        // router drives `tick_start` with the demodulator-recovered downlink
        // time, which advances one slot per tick, so feed an incrementing clock.
        let mut emitted_end = None;
        let mut rx_ts = TdmaTime::default();
        for _ in 0..4 {
            umac.tick_start(&mut q, rx_ts);
            rx_ts = rx_ts.add_timeslots(1);
            if let Some(m) = q.pop_front() {
                emitted_end = Some(m);
            }
            assert!(q.pop_front().is_none(), "at most one block per tick");
        }
        let msg = emitted_end.expect("MAC-END-UL emitted on its reserved slot");
        let SapMsgInner::TmvUnitdataReq(slot) = msg.msg else {
            panic!("expected TmvUnitdataReq");
        };
        assert_eq!(slot.ts, slot0.add_timeslots(4), "second block one TDMA frame later");
        assert!(slot.reserved_access);
        let mut blk1 = slot.blk1.expect("blk1").mac_block;
        assert_eq!(blk1.get_len(), SCH_F_TYPE1_BITS);
        let end_pdu = MacEndUl::from_bitbuf(&mut blk1).expect("MAC-END-UL decodes");
        assert!(end_pdu.length_ind.is_some(), "terminating PDU carries the length");
        assert!(umac.reserved_tx.is_none(), "plan complete");
        assert!(reporter.is_transmitted(), "last block marks the receipt transmitted");
    }

    /// A grant smaller than the requested number of full slots cannot carry the
    /// multi-slot transfer: nothing is transmitted, the fragment is dropped and
    /// its receipt marked discarded so the LLC retransmits (cl. 23.5.2).
    #[test]
    fn test_multislot_grant_too_small_drops() {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 1, m: 1, h: 0 };
        let reporter = TxReporter::new();
        umac.pending_fragment = Some(UplinkFragment {
            remainder: pack(&BitBuffer::from_bitstr(&"1".repeat(700))).unwrap(),
            scrambling_code: 0,
            end_kind: FragEndKind::MacFragUl { total_slots: 3 },
            tx_reporter: umac.track_reporter(Some(reporter.clone())).unwrap(),
        });
        let mut q = MessageQueue::new();

        let grant = BasicSlotgrant {
            capacity_allocation: BasicSlotgrantCapAlloc::Grant2Slots,
            granting_delay: BasicSlotgrantGrantingDelay::CapAllocAtNextOpportunity,
        };
        umac.emit_fragment_end(&mut q, Some(&grant));

        assert!(q.pop_front().is_none(), "no uplink on an undersized grant");
        assert!(umac.pending_fragment.is_none(), "fragment consumed");
        assert!(umac.reserved_tx.is_none(), "no plan installed");
        assert!(reporter.is_discarded(), "undersized grant marks the receipt discarded");
    }

    /// The reserved-slot walk steps one TDMA frame at a time (same timeslot,
    /// +4 timeslots) and skips the mandatory CLCH slot, mirroring the BS
    /// grant-opportunity stepping (cl. 23.5.2.2.2).
    #[test]
    fn test_reserved_slot_sequence_skips_clch() {
        // At m=1, the CLCH is f=18, t=2 (is_mandatory_clch). Starting the walk
        // at t=2, f=15, m=1 makes the f=18 candidate a CLCH that must be skipped.
        let first = TdmaTime { t: 2, f: 15, m: 1, h: 0 };
        assert!(TdmaTime { t: 2, f: 18, m: 1, h: 0 }.is_mandatory_clch());
        let slots = UmacMs::reserved_slot_sequence(first, 4);
        assert_eq!(slots.len(), 4);
        let fs: Vec<u8> = slots.iter().map(|s| s.f).collect();
        // f=18 (CLCH) is skipped; the walk continues into the next multiframe.
        assert_eq!(fs, vec![15, 16, 17, 1], "CLCH frame 18 skipped");
        assert!(slots.iter().all(|s| !s.is_mandatory_clch()), "no reserved slot is CLCH");
    }

    /// `compute_ul_frag_slots` sizes N = (continuations) + 1 and rejects a
    /// remainder needing more than `MAX_UL_FRAG_SLOTS` full slots.
    #[test]
    fn test_compute_ul_frag_slots() {
        let end = UmacMs::max_end_ul_sdu_bits(); // 254
        let frag = UmacMs::max_frag_ul_sdu_bits(); // 264
        assert_eq!(UmacMs::compute_ul_frag_slots(end + 1), Some(2), "just over one slot => 2 slots");
        assert_eq!(UmacMs::compute_ul_frag_slots(end + frag), Some(2));
        assert_eq!(UmacMs::compute_ul_frag_slots(end + frag + 1), Some(3));
        // Beyond MAX_UL_FRAG_SLOTS the transfer is rejected.
        let too_big = end + frag * MAX_UL_FRAG_SLOTS;
        assert_eq!(UmacMs::compute_ul_frag_slots(too_big), None);
    }

    /// If a reserved slot is missed (the downlink clock advanced past it without
    /// transmitting), the whole multi-slot plan is abandoned and its receipt
    /// discarded so the LLC/MM retransmit (cl. 23.5.2 / 22.3.2.3).
    #[test]
    fn test_missed_reserved_slot_abandons() {
        let mut umac = ms_umac();
        umac.dltime = TdmaTime { t: 1, f: 5, m: 1, h: 0 };
        let reporter = TxReporter::new();
        // Front reserved slot equals the current downlink slot, i.e. dltime + 2
        // (the uplink slot) is already 2 timeslots past it => missed.
        let remainder = FragmentBits::from_view(tetra_ms_core::types::BitView::new(&[0; 1], 1).unwrap()).unwrap();
        let receipt = umac.track_reporter(Some(reporter.clone())).unwrap();
        umac.reserved_tx = Some(ReservedTxPlan::new(remainder.view(), 1, umac.dltime, 0, receipt).unwrap());
        let mut q = MessageQueue::new();

        umac.drive_reserved_tx(&mut q);

        assert!(q.pop_front().is_none(), "missed slot emits nothing");
        assert!(umac.reserved_tx.is_none(), "plan abandoned");
        assert!(reporter.is_discarded(), "missed slot marks the receipt discarded");
    }

    // --- Uplink transfer queueing (cl. 23.5.1.4: one transfer at a time) ---

    /// Build a TMA-UNITDATA request carrying `sdu_bits` for the MS's own ISSI
    /// with `reporter` as its transmit receipt, as the LLC hands down to the MAC.
    pub(super) fn tma_uplink_req(reporter: &TxReporter, sdu_bits: &str) -> SapMsg {
        SapMsg {
            sap: Sap::TmaSap,
            src: TetraEntity::Llc,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TmaUnitdataReq(TmaUnitdataReq {
                req_handle: 0,
                pdu: BitBuffer::from_bitstr(sdu_bits),
                main_address: TetraAddress::issi(1000001),
                link_id: 0,
                endpoint_id: 0,
                pdu_priority: None,
                is_emergency: false,
                stealing_permission: false,
                subscriber_class: 0,
                air_interface_encryption: None,
                stealing_repeats_flag: None,
                data_category: None,
                chan_alloc: None,
                tx_reporter: Some(reporter.clone()),
            }),
        }
    }

    /// Regression: a second uplink handed down while the first is still in
    /// flight must be queued, not overwrite the active `pending_uplink`. The
    /// original single-slot design silently replaced the in-flight transfer,
    /// orphaning its `TxReporter` (never marked transmitted or discarded) so the
    /// LLC never set `t_umac_done` and the acknowledged basic link wedged
    /// forever (observed as endless "still blocked" when an LLC BL-ACK auto-ack
    /// interleaved with a post-registration group attach).
    #[test]
    fn test_second_uplink_is_queued_not_clobbered() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        let mut q = MessageQueue::new();

        let r1 = TxReporter::new();
        let r2 = TxReporter::new();

        // First transfer installs directly into the active slot.
        umac.rx_tma_prim(&mut q, tma_uplink_req(&r1, "0110100100011110001011010010"));
        assert!(umac.pending_uplink.is_some(), "first transfer is active");
        assert!(umac.uplink_queue.is_empty(), "nothing queued yet");

        // Second transfer arrives before the first completes: it must queue.
        umac.rx_tma_prim(&mut q, tma_uplink_req(&r2, "1001011011100001110100101101"));
        assert_eq!(umac.uplink_queue.len(), 1, "second transfer is queued");
        assert!(umac.pending_uplink.is_some(), "first transfer still active");
        // Neither receipt has been touched: the first is still awaiting its MAC
        // acknowledgement, the second has not yet contended for access.
        assert!(!r1.is_transmitted() && !r1.is_discarded(), "first receipt untouched");
        assert!(!r2.is_transmitted() && !r2.is_discarded(), "second receipt untouched");

        // The active slot must still hold the FIRST transfer (its receipt), not
        // the second: marking the active receipt transmitted marks r1, not r2.
        let active_reporter = umac
            .pending_uplink
            .as_ref()
            .unwrap()
            .tx_reporter
            .expect("active transfer carries the first receipt");
        umac.finish_receipt(Some(active_reporter), true);
        assert!(r1.is_transmitted(), "active slot holds the first transfer");
        assert!(!r2.is_transmitted(), "second transfer is untouched in the queue");

        // Emulate the first transfer completing (MAC-RESOURCE ack took it), then
        // promotion pulls the queued second transfer into the active slot.
        umac.pending_uplink = None;
        assert!(!umac.random_access.is_active());
        umac.promote_next_uplink();
        assert!(umac.uplink_queue.is_empty(), "queue drained");
        let promoted = umac
            .pending_uplink
            .as_ref()
            .expect("second transfer promoted")
            .tx_reporter
            .expect("promoted transfer carries the second receipt");
        umac.finish_receipt(Some(promoted), true);
        assert!(r2.is_transmitted(), "promoted slot holds the second transfer");
    }

    /// A queued transfer dropped on queue overflow has its receipt marked
    /// discarded so the LLC retransmits it rather than the receipt being leaked.
    #[test]
    fn test_uplink_queue_overflow_discards_oldest() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        let mut q = MessageQueue::new();

        // Occupy the active slot so every subsequent transfer queues.
        umac.rx_tma_prim(&mut q, tma_uplink_req(&TxReporter::new(), "0110100100011110"));

        // Fill the queue to capacity; the oldest queued receipt is the canary.
        let oldest = TxReporter::new();
        umac.rx_tma_prim(&mut q, tma_uplink_req(&oldest, "0110100100011110"));
        for _ in 1..MAX_UPLINK_QUEUE {
            umac.rx_tma_prim(&mut q, tma_uplink_req(&TxReporter::new(), "0110100100011110"));
        }
        assert_eq!(umac.uplink_queue.len(), MAX_UPLINK_QUEUE, "queue at capacity");
        assert!(!oldest.is_discarded(), "oldest still queued");

        // One more transfer overflows the queue, evicting the oldest.
        umac.rx_tma_prim(&mut q, tma_uplink_req(&TxReporter::new(), "0110100100011110"));
        assert_eq!(umac.uplink_queue.len(), MAX_UPLINK_QUEUE, "queue stays capped");
        assert!(
            oldest.is_discarded(),
            "evicted transfer's receipt marked discarded (LLC retransmits)"
        );
    }

    // --- MS MAC receive filtering (ETSI TS 100 392-2 clause 23 addressing) ---

    /// Build an MS UMAC with the given own ISSI and attached groups.
    pub(super) fn umac_cfg(mode: &str, issi: u32, groups: &[u32]) -> UmacMs {
        let groups_str = groups.iter().map(u32::to_string).collect::<Vec<_>>().join(", ");
        let ms_section = if mode == "Ms" {
            format!("\n[ms]\nissi = {issi}\nsubscriber_class = 1\nattach_groups = [{groups_str}]\n")
        } else {
            String::new()
        };
        let toml = format!(
            r#"
config_version = "0.6"
stack_mode = "{mode}"

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
{ms_section}"#
        );
        let cfg = from_toml_str(&toml).expect("valid test config");
        UmacMs::new(SharedConfig::from_parts(cfg, None))
    }

    /// A generic (untyped) MAC-layer SSI address, as produced by the
    /// MAC-RESOURCE parser for both individual and group identities.
    fn ssi_addr(ssi: u32) -> TetraAddress {
        TetraAddress {
            ssi,
            ssi_type: SsiType::Ssi,
        }
    }

    #[test]
    fn test_ms_accepts_own_issi() {
        let umac = umac_cfg("Ms", 1000001, &[]);
        assert!(umac.accept_downlink_address(&ssi_addr(1000001)));
    }

    #[test]
    fn test_ms_drops_foreign_issi() {
        let umac = umac_cfg("Ms", 1000001, &[]);
        // Traffic addressed to another subscriber must be ignored.
        assert!(!umac.accept_downlink_address(&ssi_addr(2200699)));
    }

    #[test]
    fn test_ms_accepts_attached_group_only() {
        let umac = umac_cfg("Ms", 1000001, &[91, 100]);
        assert!(umac.accept_downlink_address(&ssi_addr(91)));
        assert!(umac.accept_downlink_address(&ssi_addr(100)));
        // A group we have not attached to is not for us.
        assert!(!umac.accept_downlink_address(&ssi_addr(92)));
    }

    #[test]
    fn test_ms_accepts_broadcast() {
        let umac = umac_cfg("Ms", 1000001, &[]);
        assert!(umac.accept_downlink_address(&ssi_addr(BROADCAST_SSI)));
    }

    /// G1 (cl. 17.3.2 / 23.4.1.2.1): a TL-CONFIGURE carrying the MS's identities
    /// (from the MLE-IDENTITIES chain) replaces the runtime downlink address
    /// filter, so a newly-attached group is accepted and a dropped one rejected.
    #[test]
    fn test_tlmc_configure_updates_runtime_filter() {
        let mut umac = umac_cfg("Ms", 1000001, &[91]);
        // Seeded from config: own ISSI + group 91 accepted, 220 not.
        assert!(umac.accept_downlink_address(&ssi_addr(91)));
        assert!(!umac.accept_downlink_address(&ssi_addr(220)));

        // MLE reconfigures the filter: detach 91, attach 220.
        let mut q = MessageQueue::new();
        let msg = SapMsg {
            sap: Sap::TlmcSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TlmcConfigureReq(TlmcConfigureReq {
                valid_addresses: Some(TlmcValidAddress {
                    mcc: 901,
                    mnc: 9999,
                    individual_ssi: Some(1000001),
                    group_ssis: Some(vec![220]),
                }),
                ..Default::default()
            }),
        };
        umac.rx_tlmc_configure_req(&mut q, msg);

        assert!(umac.accept_downlink_address(&ssi_addr(1000001)), "own ISSI still accepted");
        assert!(umac.accept_downlink_address(&ssi_addr(220)), "newly attached group accepted");
        assert!(!umac.accept_downlink_address(&ssi_addr(91)), "detached group now rejected");
    }

    /// G1: a scrambling-only TL-CONFIGURE (no SSI members) leaves the runtime
    /// downlink address filter untouched (e.g. at cell selection).
    #[test]
    fn test_tlmc_configure_scrambling_only_preserves_filter() {
        let mut umac = umac_cfg("Ms", 1000001, &[91]);
        let mut q = MessageQueue::new();
        let msg = SapMsg {
            sap: Sap::TlmcSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TlmcConfigureReq(TlmcConfigureReq {
                valid_addresses: Some(TlmcValidAddress {
                    mcc: 901,
                    mnc: 9999,
                    individual_ssi: None,
                    group_ssis: None,
                }),
                ..Default::default()
            }),
        };
        umac.rx_tlmc_configure_req(&mut q, msg);
        assert!(umac.accept_downlink_address(&ssi_addr(91)), "group filter preserved");
        assert!(umac.accept_downlink_address(&ssi_addr(1000001)), "own ISSI preserved");
    }

    /// D-2 (tune plumbing): a TLMC-TUNE from the MLE is forwarded down to LMAC as
    /// a TMV-TUNE carrying the same carrier (UMAC holds no radio state).
    #[test]
    fn test_tlmc_tune_forwarded_to_lmac() {
        let mut umac = umac_cfg("Ms", 1000001, &[]);
        let mut q = MessageQueue::new();
        let msg = SapMsg {
            sap: Sap::TlmcSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TlmcTuneReq(tetra_saps::tlmc::TlmcTuneReq { carrier_hz: 396_000_000 }),
        };
        umac.rx_tlmc_prim(&mut q, msg);

        let out = q.pop_front().expect("a TMV-TUNE must be emitted");
        assert_eq!(out.sap, Sap::TmvSap);
        assert_eq!(out.dest, TetraEntity::Lmac);
        let SapMsgInner::TmvTuneReq(req) = out.msg else {
            panic!("expected TmvTuneReq");
        };
        assert_eq!(req.carrier_hz, 396_000_000);
    }

    /// A SYSINFO carrying the given RF/duplex parameters (only the frequency
    /// fields matter for uplink derivation; the rest are benign zero defaults).
    fn sysinfo_rf(band: u8, carrier: u16, duplex_idx: u8, reverse: bool) -> mac::downlink::Sysinfo {
        mac::downlink::Sysinfo {
            main_carrier: carrier,
            freq_band: band,
            freq_offset_index: 0,
            duplex_spacing: duplex_idx,
            reverse_operation: reverse,
            hyperframe: None,
            default_access: None,
            option: 0,
            option_bits: 0,
            sdu_offset: 82,
        }
    }

    /// An MS UMAC whose programmed duplex table overrides index 7 with a 9.4 MHz
    /// split (matches the serving BS), so a SYSINFO advertising duplex index 7
    /// resolves to a known uplink.
    fn umac_with_duplex_table() -> UmacMs {
        let toml = r#"
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

[duplex_table]
overrides = [[7, 9400000]]

[ms]
issi = 1000001
subscriber_class = 1
attach_groups = []
"#;
        let cfg = from_toml_str(toml).expect("valid test config");
        UmacMs::new(SharedConfig::from_parts(cfg, None))
    }

    /// Camp-time uplink derivation: a SYSINFO advertising band 4 / carrier 1593 /
    /// duplex index 7 (resolved via the programmed duplex table to 9.4 MHz)
    /// yields DL 439.825 MHz, UL 430.425 MHz and emits exactly one TMV-TX-TUNE to
    /// LMAC (EN 300 392-2 cl. 18.4.2.2 / 21.4.4).
    #[test]
    fn test_uplink_derived_from_sysinfo() {
        let mut umac = umac_with_duplex_table();
        let mut q = MessageQueue::new();

        umac.maybe_retune_uplink(&mut q, &sysinfo_rf(4, 1593, 7, false));

        let tunes: Vec<u32> = q
            .iter()
            .filter_map(|m| match &m.msg {
                SapMsgInner::TmvTxTuneReq(p) => {
                    assert_eq!(m.dest, TetraEntity::Lmac);
                    assert_eq!(m.sap, Sap::TmvSap);
                    Some(p.carrier_hz)
                }
                _ => None,
            })
            .collect();
        assert_eq!(tunes, vec![430_425_000], "derived uplink carrier");
        assert_eq!(umac.derived_ul_freq, Some(430_425_000));
    }

    /// The retune is issued only when the derived uplink changes: a second,
    /// identical SYSINFO must not re-emit a TMV-TX-TUNE (avoids retune spam on
    /// every SYSINFO broadcast).
    #[test]
    fn test_uplink_derivation_deduplicated() {
        let mut umac = umac_with_duplex_table();
        let mut q1 = MessageQueue::new();
        umac.maybe_retune_uplink(&mut q1, &sysinfo_rf(4, 1593, 7, false));
        assert!(q1.iter().any(|m| matches!(m.msg, SapMsgInner::TmvTxTuneReq(_))));

        let mut q2 = MessageQueue::new();
        umac.maybe_retune_uplink(&mut q2, &sysinfo_rf(4, 1593, 7, false));
        assert!(
            q2.iter().all(|m| !matches!(m.msg, SapMsgInner::TmvTxTuneReq(_))),
            "unchanged uplink must not retune again"
        );
    }

    /// A malformed SYSINFO (out-of-range band/carrier) is ignored rather than
    /// panicking the frequency-derivation math.
    #[test]
    fn test_uplink_derivation_rejects_bad_sysinfo() {
        let mut umac = umac_with_duplex_table();
        let mut q = MessageQueue::new();
        umac.maybe_retune_uplink(&mut q, &sysinfo_rf(15, 4095, 7, false));
        assert!(q.iter().all(|m| !matches!(m.msg, SapMsgInner::TmvTxTuneReq(_))));
        assert_eq!(umac.derived_ul_freq, None, "no uplink derived from bad params");
    }

    /// Build a downlink CHANNEL ALLOCATION element (cl. 21.5.2) for the given
    /// allocation type, assigned-timeslot bitmap, carrier and UL/DL direction.
    fn chan_alloc(alloc_type: ChanAllocType, ts: [bool; 4], carrier_num: u16, ul_dl: UlDlAssignment) -> ChanAllocElement {
        ChanAllocElement {
            alloc_type,
            ts_assigned: ts,
            ul_dl_assigned: ul_dl,
            clch_permission: false,
            cell_change_flag: false,
            carrier_num,
            ext: None,
            mon_pattern: 0,
            frame18_mon_pattern: Some(0),
        }
    }

    fn tmd_speech(ts: u8, data: Vec<u8>) -> SapMsg {
        SapMsg {
            sap: Sap::TmdSap,
            src: TetraEntity::Lmac,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TmdCircuitDataInd(tetra_saps::tmd::TmdCircuitDataInd {
                ts,
                data,
                bfi: false,
                usage_marker: None,
                owner_ssi: None,
            }),
        }
    }

    /// The U-plane TMD relay is gated on the AACH downlink usage marker (cl.
    /// 21.4.7.2 / 23.5.5): speech on a slot whose usage marker the serving cell
    /// assigned to one of our calls is relayed to CMCE — tagged with the marker
    /// and the owning SSI — while speech carrying any other marker is dropped.
    /// This follows the call's traffic-channel identity, independent of which
    /// physical slot the cell places it on.
    #[test]
    fn test_traffic_relay_gated_on_usage_marker() {
        let mut umac = ms_umac();

        // The serving cell bound usage marker 16 to our group GSSI 220 (learned
        // from a MAC-RESOURCE with an SSI + usage-marker address element).
        umac.active_traffic_usage_markers.insert(16, 220);

        let mut q = MessageQueue::new();

        // This slot's AACH designates Traffic(16) — our call. Speech relays.
        umac.current_traffic_usage_marker = Some(16);
        umac.rx_tmd_prim(&mut q, tmd_speech(3, vec![1, 2, 3]));

        // A slot carrying another group's marker (17) is dropped.
        umac.current_traffic_usage_marker = Some(17);
        umac.rx_tmd_prim(&mut q, tmd_speech(2, vec![9, 9, 9]));

        // A control/unallocated slot (no marker) is dropped.
        umac.current_traffic_usage_marker = None;
        umac.rx_tmd_prim(&mut q, tmd_speech(4, vec![7, 7, 7]));

        let relayed: Vec<_> = q
            .iter()
            .filter(|m| m.dest == TetraEntity::Cmce && matches!(m.msg, SapMsgInner::TmdCircuitDataInd(_)))
            .collect();
        assert_eq!(relayed.len(), 1, "only our-marker speech relayed");
        let SapMsgInner::TmdCircuitDataInd(ind) = &relayed[0].msg else {
            unreachable!()
        };
        assert_eq!(ind.ts, 3, "timeslot tag preserved");
        assert_eq!(ind.data, vec![1, 2, 3], "speech payload preserved");
        assert_eq!(ind.usage_marker, Some(16), "relayed frame tagged with its usage marker");
        assert_eq!(ind.owner_ssi, Some(220), "relayed frame tagged with the owning SSI");
    }

    /// A CHANNEL ALLOCATION on a different carrier is a cross-carrier retune,
    /// deferred to M3 — the MS must NOT act on it (no assigned-slot change, no
    /// retune). With no usage marker bound to us, speech stays gated out.
    #[test]
    fn test_channel_allocation_cross_carrier_deferred() {
        let mut umac = ms_umac();
        umac.serving_carrier_num = Some(1593);
        let own = umac.own_issi();

        umac.act_on_channel_allocation(
            &chan_alloc(
                ChanAllocType::Replace,
                [false, false, true, false],
                1600, // different carrier
                UlDlAssignment::Both,
            ),
            own,
        );
        assert_eq!(umac.assigned_traffic_slots, [false; 4], "cross-carrier allocation not followed");

        let mut q = MessageQueue::new();
        umac.current_traffic_usage_marker = Some(16);
        umac.rx_tmd_prim(&mut q, tmd_speech(3, vec![1, 2, 3]));
        assert!(
            q.iter().all(|m| !matches!(m.msg, SapMsgInner::TmdCircuitDataInd(_))),
            "no speech relayed when no usage marker is bound to us"
        );
    }

    /// M2: an "Additional" allocation adds to the assigned set rather than
    /// replacing it (cl. 21.5.2 / 14.8.17a), and an uplink-only assignment does
    /// not change the downlink decode.
    #[test]
    fn test_channel_allocation_additional_and_ul_only() {
        let mut umac = ms_umac();
        umac.serving_carrier_num = Some(1593);
        let own = umac.own_issi();

        umac.act_on_channel_allocation(
            &chan_alloc(ChanAllocType::Replace, [false, false, true, false], 1593, UlDlAssignment::Both),
            own,
        );
        umac.act_on_channel_allocation(
            &chan_alloc(ChanAllocType::Additional, [false, true, false, false], 1593, UlDlAssignment::Dl),
            own,
        );
        assert_eq!(
            umac.assigned_traffic_slots,
            [false, true, true, false],
            "Additional adds TS2 to TS3"
        );

        // An uplink-only assignment leaves the downlink assigned set untouched.
        umac.act_on_channel_allocation(
            &chan_alloc(ChanAllocType::Replace, [true, false, false, false], 1593, UlDlAssignment::Ul),
            own,
        );
        assert_eq!(
            umac.assigned_traffic_slots,
            [false, true, true, false],
            "uplink-only allocation does not change DL slots"
        );
    }

    /// cl. 14.2.4.1 single-transceiver arbitration: while engaged on an
    /// individual (own-ISSI) traffic channel, a concurrently-notified group
    /// call's CHANNEL ALLOCATION (addressed to a GSSI) must not move the MS off
    /// the private call. Once the private call releases the U-plane (owner
    /// cleared) the group's allocation is followed again (late entry).
    #[test]
    fn test_channel_allocation_individual_call_not_preempted_by_group() {
        let mut umac = ms_umac();
        umac.serving_carrier_num = Some(1593);
        let own = umac.own_issi();
        let gssi = own + 1; // any SSI that is not our ISSI

        // Engage on an individual (own-ISSI) traffic channel on TS3.
        umac.act_on_channel_allocation(
            &chan_alloc(ChanAllocType::Replace, [false, false, true, false], 1593, UlDlAssignment::Both),
            own,
        );
        assert_eq!(umac.assigned_traffic_slots, [false, false, true, false]);
        assert_eq!(umac.traffic_channel_owner, Some(own));

        // A group call's Replace allocation (GSSI, TS4) arrives — must be ignored.
        umac.act_on_channel_allocation(
            &chan_alloc(ChanAllocType::Replace, [false, false, false, true], 1593, UlDlAssignment::Both),
            gssi,
        );
        assert_eq!(
            umac.assigned_traffic_slots,
            [false, false, true, false],
            "group allocation must not preempt the engaged individual call"
        );
        assert_eq!(umac.traffic_channel_owner, Some(own), "owner unchanged");

        // The individual call releases the U-plane resource.
        umac.traffic_channel_owner = None;

        // Now the group's re-broadcast allocation (late entry) is followed.
        umac.act_on_channel_allocation(
            &chan_alloc(ChanAllocType::Replace, [false, false, false, true], 1593, UlDlAssignment::Both),
            gssi,
        );
        assert_eq!(
            umac.assigned_traffic_slots,
            [false, false, false, true],
            "group allocation followed once the private call has released"
        );
        assert_eq!(umac.traffic_channel_owner, Some(gssi));
    }

    // ─── M4a: uplink TCH/S traffic emission ────────────────────────────────

    /// Extract the emitted uplink TCH/S traffic burst (TMV-UNITDATA to LMAC),
    /// if any.
    fn extract_uplink_tchs(q: &MessageQueue) -> Option<TmvUnitdataReqSlot> {
        q.iter().find_map(|m| match &m.msg {
            SapMsgInner::TmvUnitdataReq(slot)
                if m.dest == TetraEntity::Lmac
                    && slot
                        .blk1
                        .as_ref()
                        .map(|b| b.logical_channel == LogicalChannel::TchS)
                        .unwrap_or(false) =>
            {
                Some(slot.clone())
            }
            _ => None,
        })
    }

    /// An MLE-CONFIGURE U-plane transmit configuration (cl. 17.3.3) forwarded to
    /// UMAC over the TLMC-SAP.
    fn uplane_cfg(switch_u_plane: bool, tx_grant: bool) -> SapMsg {
        SapMsg {
            sap: Sap::TlmcSap,
            src: TetraEntity::Mle,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TlmcUPlaneConfigureReq(tetra_saps::tlmc::TlmcUPlaneConfigureReq { switch_u_plane, tx_grant }),
        }
    }

    /// An uplink U-plane source frame from CC-MS (TMD-SAP, cl. 14.5.1.4).
    fn tmd_source(data: Vec<u8>) -> SapMsg {
        SapMsg {
            sap: Sap::TmdSap,
            src: TetraEntity::Cmce,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TmdCircuitDataReq(tetra_saps::tmd::TmdCircuitDataReq { ts: 0, data }),
        }
    }

    /// A camped MS holding the transmission grant on an assigned traffic slot
    /// emits exactly one TCH/S Normal-Uplink-Burst request on the paired uplink
    /// slot (dltime + 2, cl. 9.3.9), tagged with the traffic physical channel
    /// and clock-driven (not reserved-access). The slot is followed from the
    /// CHANNEL ALLOCATION record (cl. 21.5.2), never hardcoded.
    #[test]
    fn test_ms_uplink_traffic_emitted_on_granted_assigned_slot() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false]; // TS3
        umac.uplink_tx_granted = true;
        // Emit when the *paired downlink slot* (our current dltime) is the
        // assigned traffic slot ts3; the burst is physically placed 2 timeslots
        // later on air (cl. 9.3.9), which the BS subtracts back to ts3.
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 };

        let mut q = MessageQueue::new();
        umac.drive_uplink_traffic(&mut q);

        let slot = extract_uplink_tchs(&q).expect("a TCH/S uplink burst must be emitted");
        assert_eq!(
            slot.ts,
            umac.dltime.add_timeslots(2),
            "air slot tag is the assigned paired slot delayed 2 timeslots (cl. 9.3.9)"
        );
        assert_eq!(slot.ul_phy_chan, PhysicalChannel::Tp, "traffic physical channel");
        assert!(!slot.reserved_access, "continuous traffic is clock-driven, not reserved-access");
        assert!(slot.blk2.is_none(), "single full-slot traffic block");
        assert!(slot.bbk.is_none(), "no BBK/AACH on the uplink");
        let blk1 = slot.blk1.expect("blk1 present");
        assert_eq!(blk1.logical_channel, LogicalChannel::TchS);
        assert_eq!(blk1.scrambling_code, 0x1234_5678, "serving-cell scrambling code");
        assert_eq!(blk1.mac_block.get_len(), 274, "one 274-bit TCH/S type-1 frame");
    }

    /// With no floor grant the MS is silent even on its assigned traffic slot.
    #[test]
    fn test_ms_uplink_traffic_suppressed_when_not_granted() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false];
        umac.uplink_tx_granted = false;
        // Paired slot IS the assigned ts3 (so only the missing grant suppresses).
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 };

        let mut q = MessageQueue::new();
        umac.drive_uplink_traffic(&mut q);
        assert!(extract_uplink_tchs(&q).is_none(), "no traffic emitted without a transmit grant");
    }

    /// Granted, but the paired uplink slot is not one assigned to this MS: stay
    /// silent (do not step on another call's / the control timeslot).
    #[test]
    fn test_ms_uplink_traffic_suppressed_on_unassigned_slot() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [true, false, false, false]; // TS1 only
        umac.uplink_tx_granted = true;
        // Paired downlink slot is ts3, which is NOT the assigned ts1: stay silent.
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 };

        let mut q = MessageQueue::new();
        umac.drive_uplink_traffic(&mut q);
        assert!(extract_uplink_tchs(&q).is_none(), "no traffic emitted on an unassigned slot");
    }

    /// Frame 18 is the control frame — no TCH/S traffic is emitted there even on
    /// an assigned, granted slot (cl. 9.5.1c / 23.4.2.1).
    #[test]
    fn test_ms_uplink_traffic_suppressed_on_frame_18() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false];
        umac.uplink_tx_granted = true;
        // Paired slot is the assigned ts3 AND in frame 18: only the control-frame
        // rule suppresses the burst.
        umac.dltime = TdmaTime { t: 3, f: 18, m: 1, h: 0 };

        let mut q = MessageQueue::new();
        umac.drive_uplink_traffic(&mut q);
        assert!(extract_uplink_tchs(&q).is_none(), "no traffic on the control frame (18)");
    }

    /// Granted and assigned, but not yet camped (no scrambling code): cannot form
    /// a decodable burst, so stay silent.
    #[test]
    fn test_ms_uplink_traffic_suppressed_without_scrambling_code() {
        let mut umac = ms_umac();
        umac.scrambling_code = None;
        umac.assigned_traffic_slots = [false, false, true, false];
        umac.uplink_tx_granted = true;
        // Paired slot IS the assigned ts3 (so only the missing scrambling code
        // suppresses the burst).
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 };

        let mut q = MessageQueue::new();
        umac.drive_uplink_traffic(&mut q);
        assert!(extract_uplink_tchs(&q).is_none(), "no traffic emitted before camping");
    }

    /// The MLE-CONFIGURE seam drives the transmit grant: tx_grant AND
    /// switch_u_plane together arm emission; either off disarms it and flushes
    /// any buffered uplink audio so a later grant cannot emit stale frames.
    #[test]
    fn test_ms_uplane_configure_seam_sets_and_flushes_grant() {
        let mut umac = ms_umac();
        let mut q = MessageQueue::new();

        umac.mac
            .queue_audio(tetra_ms_core::types::BitView::new(&[0; 35], 274).unwrap())
            .unwrap();
        umac.rx_tlmc_prim(&mut q, uplane_cfg(true, true));
        assert!(umac.uplink_tx_granted, "grant armed when tx_grant && switch_u_plane");

        // Grant to another user (tx_grant true, U-plane still on) is NOT us.
        umac.rx_tlmc_prim(&mut q, uplane_cfg(true, false));
        assert!(!umac.uplink_tx_granted, "not the talker when tx_grant is false");
        assert!(umac.uplink_audio.is_empty(), "de-grant flushes buffered uplink audio");
    }

    /// The uplink U-plane source buffer is bounded (drop-oldest) so a CC push
    /// rate faster than the emit rate cannot grow it without bound (cl. 23).
    #[test]
    fn test_ms_uplink_audio_buffer_bounded() {
        let mut umac = ms_umac();
        let mut q = MessageQueue::new();
        umac.rx_tlmc_prim(&mut q, uplane_cfg(true, true));
        assert!(umac.uplink_tx_granted);
        for i in 0..(UPLINK_AUDIO_MAX_FRAMES + 3) {
            umac.rx_tmd_prim(&mut q, tmd_source(vec![i as u8; 35]));
        }
        assert_eq!(
            umac.uplink_audio.len(),
            UPLINK_AUDIO_MAX_FRAMES,
            "buffer capped, oldest frames dropped"
        );
        assert_eq!(umac.uplink_audio.pop_front().unwrap().view().read(0, 8), Ok(3));
    }

    /// A buffered (non-silence) source frame is clocked out on the granted slot:
    /// the emitted type-1 block carries the supplied audio, and the buffer entry
    /// is consumed.
    #[test]
    fn test_ms_uplink_emits_buffered_source_frame() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false];
        umac.uplink_tx_granted = true;
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 }; // paired slot == assigned ts3

        let mut q = MessageQueue::new();
        umac.rx_tmd_prim(&mut q, tmd_source(vec![0xFF; 35])); // distinctive, non-silence
        umac.drive_uplink_traffic(&mut q);

        let slot = extract_uplink_tchs(&q).expect("a TCH/S uplink burst must be emitted");
        let mac_block = slot.blk1.expect("blk1 present").mac_block;
        assert_eq!(mac_block.get_len(), 274);
        assert!(
            mac_block.to_bitstr().contains('1'),
            "emitted the buffered (non-silence) source frame, not synthesised silence"
        );
        assert!(umac.uplink_audio.is_empty(), "buffered frame consumed on emit");
    }

    // ─── M4b: FACCH/STCH stealing of associated signalling ─────────────────

    /// Build a TMA-UNITDATA request with a configurable stealing permission and
    /// link, carrying a short floor-control-sized SDU, as the LLC hands a floor
    /// PDU (BL-DATA) down to the MAC.
    pub(super) fn tma_stealing_req(stealing: bool, link_id: u32, reporter: Option<TxReporter>) -> SapMsg {
        SapMsg {
            sap: Sap::TmaSap,
            src: TetraEntity::Llc,
            dest: TetraEntity::Umac,
            msg: SapMsgInner::TmaUnitdataReq(TmaUnitdataReq {
                req_handle: 0,
                // ~24-bit floor PDU (U-TX-CEASED / U-TX-DEMAND are tiny).
                pdu: BitBuffer::from_bitstr(&"1".repeat(24)),
                main_address: TetraAddress::issi(1000001),
                link_id,
                endpoint_id: 0,
                pdu_priority: None,
                is_emergency: false,
                stealing_permission: stealing,
                subscriber_class: 0,
                air_interface_encryption: None,
                stealing_repeats_flag: None,
                data_category: None,
                chan_alloc: None,
                tx_reporter: reporter,
            }),
        }
    }

    /// Extract the emitted stolen uplink burst (blk1 = STCH signalling,
    /// blk2 = TCH/S speech half), if any.
    fn extract_stolen_burst(q: &MessageQueue) -> Option<TmvUnitdataReqSlot> {
        q.iter().find_map(|m| match &m.msg {
            SapMsgInner::TmvUnitdataReq(slot)
                if m.dest == TetraEntity::Lmac
                    && slot
                        .blk1
                        .as_ref()
                        .map(|b| b.logical_channel == LogicalChannel::Stch)
                        .unwrap_or(false) =>
            {
                Some(slot.clone())
            }
            _ => None,
        })
    }

    /// When a traffic channel is assigned and L3 requests stealing, the floor PDU
    /// is queued for FACCH/STCH stealing on the TCH — NOT sent by MCCH random
    /// access. `rx_tma_prim` neither emits a burst nor sets a random-access
    /// pending transfer; it parks the block in `pending_stolen_signalling`.
    #[test]
    fn test_ms_stealing_request_queues_on_assigned_tch() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false]; // TS3 assigned

        let mut q = MessageQueue::new();
        umac.rx_tma_prim(&mut q, tma_stealing_req(true, 2, None));

        assert_eq!(umac.pending_stolen_signalling.len(), 1, "floor PDU queued for stealing");
        assert!(umac.pending_uplink.is_none(), "no MCCH random-access transfer started");
        assert_eq!(q.iter().count(), 0, "nothing emitted from rx_tma_prim itself");
    }

    /// Talker stealing: granted + assigned + a queued floor PDU → the granted
    /// slot is stolen. blk1 carries the STCH signalling MAC block (a 124-bit
    /// type-1 block), blk2 the remaining TCH/S speech half; the LMAC will select
    /// normal training sequence 2 from the STCH channel.
    #[test]
    fn test_ms_stealing_emits_stolen_burst_when_talker() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false];
        umac.uplink_tx_granted = true;
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 }; // paired slot == assigned ts3

        let mut q = MessageQueue::new();
        umac.rx_tma_prim(&mut q, tma_stealing_req(true, 2, None));
        umac.drive_uplink_traffic(&mut q);

        let slot = extract_stolen_burst(&q).expect("a stolen STCH burst must be emitted");
        assert_eq!(
            slot.ts,
            umac.dltime.add_timeslots(2),
            "stolen on the assigned paired slot, air-placed 2 timeslots later (cl. 9.3.9)"
        );
        assert_eq!(slot.ul_phy_chan, PhysicalChannel::Tp, "traffic physical channel");
        let blk1 = slot.blk1.expect("blk1 (STCH) present");
        assert_eq!(blk1.logical_channel, LogicalChannel::Stch);
        assert_eq!(blk1.mac_block.get_len(), 124, "STCH type-1 half-slot block");
        assert_eq!(blk1.scrambling_code, 0x1234_5678);
        let blk2 = slot.blk2.expect("blk2 (TCH/S speech half) present");
        assert_eq!(blk2.logical_channel, LogicalChannel::TchS);
        assert_eq!(blk2.mac_block.get_len(), 274, "TCH/S speech type-1 frame");
        assert!(umac.pending_stolen_signalling.is_empty(), "queued block consumed");
    }

    /// Listener stealing (U-TX-DEMAND while not the talker): a floor PDU can be
    /// stolen on the assigned uplink slot even without the transmission grant —
    /// the speech half is silence. This is how the MS requests the floor mid-call
    /// over the TCH rather than the MCCH.
    #[test]
    fn test_ms_stealing_emits_stolen_burst_when_listening() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false];
        umac.uplink_tx_granted = false; // listening, not the talker
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 }; // paired slot == assigned ts3

        let mut q = MessageQueue::new();
        umac.rx_tma_prim(&mut q, tma_stealing_req(true, 2, None));
        umac.drive_uplink_traffic(&mut q);

        let slot = extract_stolen_burst(&q).expect("a stolen STCH burst must be emitted while listening");
        let blk1 = slot.blk1.expect("blk1 (STCH) present");
        assert_eq!(blk1.logical_channel, LogicalChannel::Stch);
        let blk2 = slot.blk2.expect("blk2 (silence speech half) present");
        assert_eq!(blk2.logical_channel, LogicalChannel::TchS);
        assert_eq!(blk2.mac_block.get_len(), 274, "silence TCH/S frame fills the speech half");
    }

    /// The stolen block's LLC transmit receipt is marked transmitted when the
    /// block is actually emitted (cl. 22.3.2.3), so the acknowledged-mode basic
    /// link progresses rather than wedging.
    #[test]
    fn test_ms_stealing_marks_tx_reporter_on_emit() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false, false, true, false];
        umac.uplink_tx_granted = true;
        umac.dltime = TdmaTime { t: 3, f: 1, m: 1, h: 0 }; // paired slot == assigned ts3

        let reporter = TxReporter::new();
        let mut q = MessageQueue::new();
        umac.rx_tma_prim(&mut q, tma_stealing_req(true, 2, Some(reporter.clone())));
        assert!(!reporter.is_transmitted(), "not yet transmitted while queued");

        umac.drive_uplink_traffic(&mut q);
        assert!(reporter.is_transmitted(), "receipt marked transmitted on emit");
    }

    /// Fallback: with NO traffic channel assigned (pre-TCH floor request), a
    /// stealing request cannot steal a slot, so it goes over the MCCH by random
    /// access (a MAC-ACCESS on SCH/HU) — nothing is queued for stealing.
    #[test]
    fn test_ms_stealing_falls_back_to_mcch_without_tch() {
        let mut umac = ms_umac();
        umac.scrambling_code = Some(0x1234_5678);
        umac.assigned_traffic_slots = [false; 4]; // no TCH assigned

        let mut q = MessageQueue::new();
        umac.rx_tma_prim(&mut q, tma_stealing_req(true, 2, None));

        assert!(umac.pending_stolen_signalling.is_empty(), "nothing to steal without a TCH");
        let pending = umac.pending_uplink.as_ref().expect("MCCH random-access transfer started");
        assert_eq!(pending.logical_channel, Channel::SchHu, "carried on SCH/HU (MCCH access)");
    }
}
