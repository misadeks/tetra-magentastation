//! Authoritative MS random access; entropy and decoded broadcast fields are caller-owned.
use tetra_ms_types::TdmaTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessAssignUlUsage {
    AssignedOnly,
    Other,
}
#[derive(Debug, Clone, Copy)]
pub struct AccessField {
    pub access_code: u8,
    pub base_frame_len: u8,
}
#[derive(Debug, Clone, Copy)]
pub struct AccessAssign {
    pub ul_usage: AccessAssignUlUsage,
    pub f1_af1: Option<AccessField>,
    pub f2_af2: Option<AccessField>,
    pub f2_af: Option<AccessField>,
}

/// The four random access codes (ETSI TS 100 392-2 Table 21.85, "Access code").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AccessCode {
    A = 0,
    B = 1,
    C = 2,
    D = 3,
}

impl AccessCode {
    /// Map a raw 2-bit access code value to the enum.
    pub fn from_raw(v: u8) -> Option<Self> {
        match v {
            0 => Some(AccessCode::A),
            1 => Some(AccessCode::B),
            2 => Some(AccessCode::C),
            3 => Some(AccessCode::D),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// Interpretation of the ACCESS-ASSIGN "Base frame-length" sub-field
/// (ETSI TS 100 392-2 Table 21.85).
///
/// The raw 4-bit value does not map linearly to a subslot count beyond 5
/// subslots, so the mapping is taken verbatim from the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseFrameLength {
    /// `0000` — reserved subslot (access code has no meaning).
    ReservedSubslot,
    /// `0001` — CLCH(-Q) subslot, i.e. subslot 1 available for linearization
    /// only (access code has no meaning).
    ClchSubslot,
    /// `0010` — ongoing frame (no new frame marker here).
    OngoingFrame,
    /// `0011`..=`1111` — a frame marker starting a new access frame whose base
    /// length is the given number of subslots.
    FrameMarker(u16),
}

impl BaseFrameLength {
    /// Decode the 4-bit base frame-length field per Table 21.85.
    pub fn from_raw(v: u8) -> Self {
        match v & 0xF {
            0b0000 => BaseFrameLength::ReservedSubslot,
            0b0001 => BaseFrameLength::ClchSubslot,
            0b0010 => BaseFrameLength::OngoingFrame,
            0b0011 => BaseFrameLength::FrameMarker(1),
            0b0100 => BaseFrameLength::FrameMarker(2),
            0b0101 => BaseFrameLength::FrameMarker(3),
            0b0110 => BaseFrameLength::FrameMarker(4),
            0b0111 => BaseFrameLength::FrameMarker(5),
            0b1000 => BaseFrameLength::FrameMarker(6),
            0b1001 => BaseFrameLength::FrameMarker(8),
            0b1010 => BaseFrameLength::FrameMarker(10),
            0b1011 => BaseFrameLength::FrameMarker(12),
            0b1100 => BaseFrameLength::FrameMarker(16),
            0b1101 => BaseFrameLength::FrameMarker(20),
            0b1110 => BaseFrameLength::FrameMarker(24),
            0b1111 => BaseFrameLength::FrameMarker(32),
            _ => unreachable!(),
        }
    }

    /// True if this field starts a new access frame (a frame marker of ≥ 1
    /// subslots), per cl. 23.5.1.4.6.
    pub fn is_frame_marker(&self) -> bool {
        matches!(self, BaseFrameLength::FrameMarker(_))
    }

    /// The base frame-length in subslots for a frame marker, or `None` for the
    /// special values (reserved / CLCH / ongoing frame).
    pub fn base_subslots(&self) -> Option<u16> {
        match self {
            BaseFrameLength::FrameMarker(n) => Some(*n),
            _ => None,
        }
    }

    /// True if the subslot is reserved (or, for CLCH, unavailable for a normal
    /// access request), so it must not be counted as an access opportunity
    /// (cl. 23.5.1.4.7 point e).
    pub fn is_reserved(&self) -> bool {
        matches!(self, BaseFrameLength::ReservedSubslot | BaseFrameLength::ClchSubslot)
    }
}

/// Random access parameters advertised for one access code (the "ALOHA
/// parameters" of ETSI TS 100 392-2 cl. 23.5.1.4.1). These come either from an
/// ACCESS-DEFINE PDU (cl. 21.4.4.3) or from the SYSINFO default definition for
/// access code A (cl. 21.4.4.1); both carry the same core fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomAccessParams {
    /// IMM: `0` = always randomize; `15` = immediate access allowed; otherwise
    /// randomize after up to IMM TDMA frames (cl. 23.5.1.4.5).
    pub imm: u8,
    /// WT: number of downlink signalling opportunities to wait for a response
    /// before assuming failure (cl. 23.5.1.4.8).
    pub wt: u8,
    /// Nu: maximum number of random access transmissions (cl. 23.5.1.4.9).
    /// `Nu == 0` means the access code is not available for use
    /// (cl. 23.5.1.4.1).
    pub nu: u8,
    /// Frame-length factor: if true, multiply the base frame-length by 4
    /// (cl. 23.5.1.4.6).
    pub fl_factor: bool,
    /// Timeslot pointer: which uplink slots are potential access opportunities
    /// (cl. 23.5.1.4.1). `0` means "same as downlink slot assignment".
    pub ts_ptr: u8,
    /// Minimum valid PDU priority for this access code (cl. 23.5.1.4.4 a).
    pub min_pdu_prio: u8,
    /// Optional subscriber class restriction bitmap (cl. 23.5.1.4.4 b); `None`
    /// means no subscriber class restriction.
    pub subscriber_class: Option<u16>,
    /// Optional group address (GSSI) restriction (cl. 23.5.1.4.4 c); `None`
    /// means no address restriction.
    pub gssi: Option<u32>,
}

impl RandomAccessParams {
    pub fn validate(&self) -> Result<(), RaFailure> {
        if self.imm > 15
            || self.wt > 15
            || self.nu > 15
            || self.ts_ptr > 15
            || self.min_pdu_prio > 7
            || self.gssi.is_some_and(|ssi| ssi > 0xff_ffff)
        {
            return Err(RaFailure::InvalidParameters);
        }
        Ok(())
    }
    /// Whether this access code is currently available for use. `Nu == 0`
    /// indicates the code is not available (cl. 23.5.1.4.1).
    pub fn is_available(&self) -> bool {
        self.nu != 0
    }
}

/// Store of the serving cell's random access parameters, per access code, with
/// the SYSINFO-vs-ACCESS-DEFINE precedence of cl. 23.5.1.4.10.
///
/// For access code A: the SYSINFO default definition is used until a "common"
/// ACCESS-DEFINE PDU for access code A is received; thereafter ACCESS-DEFINE
/// takes over and SYSINFO defaults are ignored. Access codes B, C, D are only
/// available once their ACCESS-DEFINE PDU has been received.
#[derive(Debug, Default, Clone)]
pub struct AccessParamStore {
    /// ACCESS-DEFINE parameters per access code (index 0..=3 = A..=D).
    defined: [Option<RandomAccessParams>; 4],
    /// SYSINFO default definition for access code A (cl. 21.4.4.1).
    sysinfo_default_a: Option<RandomAccessParams>,
    /// Whether a "common" ACCESS-DEFINE for access code A has been received; it
    /// permanently overrides the SYSINFO default (cl. 23.5.1.4.10).
    access_define_a_received: bool,
}

impl AccessParamStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt the SYSINFO default definition for access code A (cl. 21.4.4.1).
    /// Ignored once an ACCESS-DEFINE for code A has been received
    /// (cl. 23.5.1.4.10).
    pub fn update_sysinfo_default_a(&mut self, params: RandomAccessParams) {
        self.sysinfo_default_a = Some(params);
    }

    /// Adopt an ACCESS-DEFINE PDU (cl. 21.4.4.3) for the access code it defines.
    /// A subsequent ACCESS-DEFINE for the same code overwrites the previous
    /// definition (cl. 23.5.1.4.1 NOTE 1).
    pub fn update_access_define(&mut self, code: AccessCode, params: RandomAccessParams) {
        if code == AccessCode::A {
            self.access_define_a_received = true;
        }
        self.defined[code.index()] = Some(params);
    }

    /// The currently valid parameters for an access code, applying the
    /// SYSINFO/ACCESS-DEFINE precedence (cl. 23.5.1.4.10). Returns `None` if the
    /// code has no valid definition yet.
    pub fn params_for(&self, code: AccessCode) -> Option<&RandomAccessParams> {
        match code {
            AccessCode::A => {
                if self.access_define_a_received {
                    self.defined[AccessCode::A.index()].as_ref()
                } else {
                    self.sysinfo_default_a.as_ref()
                }
            }
            other => self.defined[other.index()].as_ref(),
        }
    }
}

/// The access opportunity state of one uplink subslot, derived from an
/// ACCESS-ASSIGN Access field (ETSI TS 100 392-2 cl. 21.5.1 / 23.5.1.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubslotAccess {
    /// The designated access code, if any. Meaningful only for "ongoing frame"
    /// and frame-marker subslots; for reserved/CLCH subslots the access code has
    /// no meaning (Table 21.85 note 1) and this is `None`.
    pub access_code: Option<AccessCode>,
    /// The base frame-length interpretation of this subslot.
    pub frame_len: BaseFrameLength,
}

impl SubslotAccess {
    /// A reserved subslot (not usable for access).
    pub fn reserved() -> Self {
        SubslotAccess {
            access_code: None,
            frame_len: BaseFrameLength::ReservedSubslot,
        }
    }

    /// Whether this subslot is a usable access opportunity for `code`: it must
    /// not be reserved or a CLCH (linearization) subslot, and its designated
    /// access code must match (cl. 23.5.1.4.7 points d and e).
    pub fn is_opportunity_for(&self, code: AccessCode) -> bool {
        !self.frame_len.is_reserved() && self.access_code == Some(code)
    }

    /// Whether this subslot starts a new access frame for `code`, i.e. carries a
    /// frame marker for that access code (cl. 23.5.1.4.6).
    pub fn is_frame_marker_for(&self, code: AccessCode) -> bool {
        self.frame_len.is_frame_marker() && self.access_code == Some(code)
    }
}

/// The access rights conveyed by one ACCESS-ASSIGN PDU to the two subslots of
/// the corresponding uplink slot (ETSI TS 100 392-2 cl. 23.5.1.4.2). The uplink
/// slot is two timeslots after the downlink slot that carried the ACCESS-ASSIGN
/// (cl. 9, note in 23.5.1.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlotAccessAssign {
    pub subslot1: SubslotAccess,
    pub subslot2: SubslotAccess,
}

impl SlotAccessAssign {
    fn both_reserved() -> Self {
        SlotAccessAssign {
            subslot1: SubslotAccess::reserved(),
            subslot2: SubslotAccess::reserved(),
        }
    }
}

fn subslot_from_field(af: &AccessField) -> SubslotAccess {
    SubslotAccess {
        access_code: AccessCode::from_raw(af.access_code),
        frame_len: BaseFrameLength::from_raw(af.base_frame_len),
    }
}

/// Interpret an ACCESS-ASSIGN PDU into the access rights for the two uplink
/// subslots, per ETSI TS 100 392-2 cl. 23.5.1.4.2. `on_common_channel` is true
/// when the MS is camped on its common control channel (currently always true
/// for this MS implementation).
///
/// Rules implemented (cl. 23.5.1.4.2):
/// - An MS on the CCCH treats an "Assigned only" uplink designation as both
///   subslots reserved.
/// - Two access fields: field 1 → subslot 1, field 2 → subslot 2, independently.
/// - A single access field applies to both subslots per points a)–e):
///   reserved → both reserved; CLCH → subslot 1 linearization, subslot 2
///   reserved; ongoing frame → both ongoing; frame marker → subslot 1 marker,
///   subslot 2 ongoing frame.
/// - No access field (e.g. uplink traffic) → both subslots reserved.
pub fn interpret_access_assign(aa: &AccessAssign, on_common_channel: bool) -> SlotAccessAssign {
    // An MS on its common control channel regards an "Assigned only" uplink slot
    // as reserved (cl. 23.5.1.4.2).
    if on_common_channel && aa.ul_usage == AccessAssignUlUsage::AssignedOnly {
        return SlotAccessAssign::both_reserved();
    }

    // Two access fields present: independent rights per subslot (header == 0).
    if let (Some(af1), Some(af2)) = (aa.f1_af1, aa.f2_af2) {
        return SlotAccessAssign {
            subslot1: subslot_from_field(&af1),
            subslot2: subslot_from_field(&af2),
        };
    }

    // A single access field applies to both subslots per points a)–e).
    if let Some(af) = aa.f2_af {
        let code = AccessCode::from_raw(af.access_code);
        let fl = BaseFrameLength::from_raw(af.base_frame_len);
        let (s1, s2) = match fl {
            BaseFrameLength::ReservedSubslot => (SubslotAccess::reserved(), SubslotAccess::reserved()),
            BaseFrameLength::ClchSubslot => (
                SubslotAccess {
                    access_code: None,
                    frame_len: BaseFrameLength::ClchSubslot,
                },
                SubslotAccess::reserved(),
            ),
            BaseFrameLength::OngoingFrame => {
                let s = SubslotAccess {
                    access_code: code,
                    frame_len: BaseFrameLength::OngoingFrame,
                };
                (s, s)
            }
            BaseFrameLength::FrameMarker(_) => (
                SubslotAccess {
                    access_code: code,
                    frame_len: fl,
                },
                SubslotAccess {
                    access_code: code,
                    frame_len: BaseFrameLength::OngoingFrame,
                },
            ),
        };
        return SlotAccessAssign {
            subslot1: s1,
            subslot2: s2,
        };
    }

    // No access field (uplink traffic) → both subslots reserved.
    SlotAccessAssign::both_reserved()
}

/// Which of the two subslots of an uplink slot to transmit in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subslot {
    One,
    Two,
}

/// Caller-supplied entropy, never a platform callback.
#[derive(Debug, Clone, Copy)]
pub struct RaChoices {
    index: u16,
    subslot: Subslot,
    requested: Option<EntropyRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntropyRequest {
    SubslotIndex(u16),
    OneOfTwo,
}

impl RaChoices {
    pub fn new(index: u16, subslot: Subslot) -> Self {
        Self {
            index,
            subslot,
            requested: None,
        }
    }
    fn choose_subslot_index(&mut self, length: u16) -> u16 {
        self.requested = Some(EntropyRequest::SubslotIndex(length));
        self.index
    }
    fn choose_one_of_two(&mut self) -> Subslot {
        self.requested = Some(EntropyRequest::OneOfTwo);
        self.subslot
    }
}

/// Reason a random access attempt was abandoned (ETSI TS 100 392-2
/// cl. 23.5.1.4.9), reported back to the requesting layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RaFailure {
    InvalidTime,
    InvalidParameters,
    InvalidEntropy,
    Busy,
    /// The access code has no valid parameters or is not available (Nu == 0)
    /// (cl. 23.5.1.4.1).
    NoValidAccessCode,
    /// The PDU is not permitted to use this access code (cl. 23.5.1.4.4).
    AccessCodeNotPermitted,
    /// The maximum number of transmissions (Nu, or 2·Nu for emergency) was
    /// reached without a response (cl. 23.5.1.4.9).
    MaxTransmissions,
}

/// An action the random access state machine asks its owner (UMAC) to perform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RaAction {
    /// Transmit the pending MAC-ACCESS in the given uplink slot/subslot. The
    /// uplink slot is two timeslots after the downlink slot that produced this
    /// action (cl. 9, note in 23.5.1.4.2).
    Transmit { ul_time: TdmaTime, subslot: Subslot },
    /// The attempt succeeded: a matching response was received (cl. 23.5.1.4.8).
    Succeeded,
    /// The attempt was abandoned (cl. 23.5.1.4.9).
    Failed(RaFailure),
}

/// Internal state of the random access attempt (ETSI TS 100 392-2
/// cl. 23.5.1.4.5–.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RaState {
    /// No attempt in progress.
    Idle,
    /// First-try window: transmit in the first valid access opportunity subject
    /// to the IMM rule (cl. 23.5.1.4.5). `init_frame` is the absolute TDMA frame
    /// index at which the attempt was initiated.
    FirstTry { init_frame: i32 },
    /// Waiting for the next frame marker for the access code to start a new
    /// access frame (cl. 23.5.1.4.6).
    AwaitFrameMarker,
    /// Counting access opportunities within the chosen access frame toward the
    /// randomly chosen subslot `chosen` (cl. 23.5.1.4.6/.4.7). `counted` is the
    /// number of qualifying subslots seen so far.
    Counting { chosen: u16, counted: u16 },
    /// Transmitted; waiting up to WT downlink opportunities for a response
    /// (cl. 23.5.1.4.8). `wt_seen` counts elapsed downlink opportunities and
    /// `last_frame` is the last frame index counted (one opportunity per frame).
    AwaitResponse { wt_seen: u8, last_frame: i32 },
}

/// MS random access state machine (ETSI TS 100 392-2 cl. 23.5.1.4).
///
/// The state machine is MS-local and pure: it is driven per received downlink
/// slot via [`MsRandomAccess::poll_downlink_slot`] with the interpreted
/// ACCESS-ASSIGN for that slot, and informed of responses via
/// [`MsRandomAccess::on_mac_resource`]. It never transmits itself; it returns
/// [`RaAction`]s for the owner (UMAC) to carry out.
///
/// One attempt is handled at a time (cl. 23.5.1.4.3): a new attempt is only
/// started via [`MsRandomAccess::initiate`] while idle.
#[derive(Debug, Clone)]
pub struct MsRandomAccess {
    state: RaState,
    /// The access code chosen for the current attempt.
    code: AccessCode,
    /// Whether the pending PDU is an emergency (priority 7), which relaxes the
    /// first-try and doubles the transmission limit (cl. 23.5.1.4.4/.4.5/.4.9).
    is_emergency: bool,
    /// Number of random access transmissions performed in this attempt
    /// (cl. 23.5.1.4.9).
    tx_count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessPhase {
    Idle,
    FirstTry,
    AwaitFrameMarker,
    Counting,
    AwaitResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessStatus {
    pub phase: AccessPhase,
    pub code: Option<AccessCode>,
    pub emergency: bool,
    pub transmissions: u8,
}

impl Default for MsRandomAccess {
    fn default() -> Self {
        MsRandomAccess {
            state: RaState::Idle,
            code: AccessCode::A,
            is_emergency: false,
            tx_count: 0,
        }
    }
}

/// The absolute TDMA frame index of a time (monotonic, 4 timeslots per frame).
fn frame_index(t: TdmaTime) -> i32 {
    t.to_int().div_euclid(4)
}

/// The access frame length in subslots for a frame marker, applying the
/// frame-length factor (cl. 23.5.1.4.6): base × 4 if the factor is set, else
/// base × 1.
fn frame_length(base_subslots: u16, fl_factor: bool) -> u16 {
    base_subslots.saturating_mul(if fl_factor { 4 } else { 1 }).max(1)
}

impl MsRandomAccess {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn status(&self) -> AccessStatus {
        AccessStatus {
            phase: match self.state {
                RaState::Idle => AccessPhase::Idle,
                RaState::FirstTry { .. } => AccessPhase::FirstTry,
                RaState::AwaitFrameMarker => AccessPhase::AwaitFrameMarker,
                RaState::Counting { .. } => AccessPhase::Counting,
                RaState::AwaitResponse { .. } => AccessPhase::AwaitResponse,
            },
            code: self.active_code(),
            emergency: self.is_active() && self.is_emergency,
            transmissions: if self.is_active() { self.tx_count } else { 0 },
        }
    }

    /// True while a random access attempt is in progress (cl. 23.5.1.4.3: only
    /// one attempt at a time).
    pub fn is_active(&self) -> bool {
        self.state != RaState::Idle
    }

    /// The access code of the in-progress attempt (for response matching).
    pub fn active_code(&self) -> Option<AccessCode> {
        if self.is_active() { Some(self.code) } else { None }
    }

    /// Initiate a random access attempt for a pending MAC-ACCESS PDU
    /// (cl. 23.5.1.4.3). `now` is the current downlink time, `params` the access
    /// parameters in force for `code` (cl. 23.5.1.4.10), `pdu_prio` the PDU
    /// priority (0..=7), and `is_emergency` whether it is an emergency PDU
    /// (priority 7, cl. 23.5.1.4.4/.4.5).
    ///
    /// Performs the access-code permission checks of cl. 23.5.1.4.4 (PDU
    /// priority ≥ minimum, subject to the emergency exception) and returns
    /// `Err(RaFailure)` if the code is unavailable or not permitted. On success
    /// the attempt is armed and subsequent `poll_downlink_slot` calls drive it.
    pub fn initiate(
        &mut self,
        now: TdmaTime,
        code: AccessCode,
        params: &RandomAccessParams,
        pdu_prio: u8,
        is_emergency: bool,
    ) -> Result<(), RaFailure> {
        if !now.is_valid() {
            return Err(RaFailure::InvalidTime);
        }
        params.validate()?;
        if pdu_prio > 7 {
            return Err(RaFailure::InvalidParameters);
        }
        if self.is_active() {
            return Err(RaFailure::Busy);
        }
        // cl. 23.5.1.4.1: Nu == 0 → access code not available.
        if !params.is_available() {
            return Err(RaFailure::NoValidAccessCode);
        }
        // cl. 23.5.1.4.4: an emergency PDU may use access code A without the
        // priority check; otherwise the PDU priority must meet the minimum.
        let emergency_code_a = is_emergency && code == AccessCode::A;
        if !emergency_code_a && pdu_prio < params.min_pdu_prio {
            return Err(RaFailure::AccessCodeNotPermitted);
        }

        self.code = code;
        self.is_emergency = is_emergency;
        self.tx_count = 0;
        // cl. 23.5.1.4.5: IMM == 0 always randomizes (go straight to a new
        // access frame); otherwise start the first-try window.
        self.state = if params.imm == 0 && !is_emergency {
            RaState::AwaitFrameMarker
        } else {
            RaState::FirstTry {
                init_frame: frame_index(now),
            }
        };
        Ok(())
    }

    /// The maximum number of random access transmissions permitted for the
    /// current attempt: Nu, doubled to 2·Nu for an emergency PDU
    /// (cl. 23.5.1.4.9).
    fn max_transmissions(&self, params: &RandomAccessParams) -> u32 {
        let nu = params.nu as u32;
        if self.is_emergency { nu.saturating_mul(2) } else { nu }
    }

    /// Arm the wait-for-response state after transmitting (cl. 23.5.1.4.8) and
    /// return the transmit action for the given downlink slot.
    fn transmit(&mut self, dltime: TdmaTime, subslot: Subslot) -> RaAction {
        self.tx_count += 1;
        self.state = RaState::AwaitResponse {
            wt_seen: 0,
            last_frame: frame_index(dltime),
        };
        RaAction::Transmit {
            // The uplink slot is two timeslots after the downlink slot that
            // conveyed the ACCESS-ASSIGN (cl. 9, note in 23.5.1.4.2).
            ul_time: dltime.add_timeslots(2),
            subslot,
        }
    }

    /// Begin a fresh transmission (re-try, cl. 23.5.1.4.8) if the maximum
    /// transmission count has not been reached, otherwise abandon
    /// (cl. 23.5.1.4.9). Returns `Some(Failed)` when abandoning.
    fn retry_or_abandon(&mut self, params: &RandomAccessParams) -> Option<RaAction> {
        if self.tx_count as u32 >= self.max_transmissions(params) {
            self.state = RaState::Idle;
            Some(RaAction::Failed(RaFailure::MaxTransmissions))
        } else {
            // cl. 23.5.1.4.8: with no response, choose a new access frame and
            // retransmit.
            self.state = RaState::AwaitFrameMarker;
            None
        }
    }

    /// Probe the bounded, cloneable state without committing a transition.
    /// The caller draws only the entropy actually requested, then submits it
    /// to `poll_downlink_slot`. No RNG or callback enters the core.
    pub fn entropy_needed(
        &self,
        dltime: TdmaTime,
        assign: &SlotAccessAssign,
        ul_slot_valid: bool,
        params: &RandomAccessParams,
    ) -> Result<Option<EntropyRequest>, RaFailure> {
        if !dltime.is_valid() {
            return Err(RaFailure::InvalidTime);
        }
        params.validate()?;
        let mut probe = self.clone();
        let mut choices = RaChoices::new(1, Subslot::One);
        probe.poll_unchecked(dltime, assign, ul_slot_valid, params, &mut choices);
        Ok(choices.requested)
    }

    pub fn poll_downlink_slot(
        &mut self,
        dltime: TdmaTime,
        assign: &SlotAccessAssign,
        ul_slot_valid: bool,
        params: &RandomAccessParams,
        choices: &mut RaChoices,
    ) -> Result<Option<RaAction>, RaFailure> {
        if let Some(EntropyRequest::SubslotIndex(length)) = self.entropy_needed(dltime, assign, ul_slot_valid, params)?
            && (choices.index == 0 || choices.index > length)
        {
            return Err(RaFailure::InvalidEntropy);
        }
        Ok(self.poll_unchecked(dltime, assign, ul_slot_valid, params, choices))
    }

    /// Drive the state machine with one received downlink slot
    /// (cl. 23.5.1.4.5–.8).
    ///
    /// - `dltime` is the downlink slot time.
    /// - `assign` is the interpreted ACCESS-ASSIGN for this slot (rights to the
    ///   uplink slot two timeslots later).
    /// - `ul_slot_valid` is true when this slot's corresponding uplink slot is a
    ///   valid access-opportunity timeslot per the timeslot pointer
    ///   (cl. 23.5.1.4.7 point a); the caller derives it from `params.ts_ptr`.
    /// - `params` are the access parameters in force for the code
    ///   (cl. 23.5.1.4.10); they may change between calls.
    /// - `rng` supplies the required random choices.
    ///
    /// Returns an [`RaAction`] to perform (transmit / abandon), or `None`.
    fn poll_unchecked(
        &mut self,
        dltime: TdmaTime,
        assign: &SlotAccessAssign,
        ul_slot_valid: bool,
        params: &RandomAccessParams,
        rng: &mut RaChoices,
    ) -> Option<RaAction> {
        match self.state {
            RaState::Idle => None,

            RaState::AwaitResponse { wt_seen, last_frame } => {
                // cl. 23.5.1.4.8: wait WT downlink opportunities for a response,
                // counting at most one opportunity per TDMA frame.
                let frame = frame_index(dltime);
                if frame != last_frame {
                    let wt_seen = wt_seen + 1;
                    if wt_seen >= params.wt {
                        // No response within WT → retransmit or abandon.
                        return self.retry_or_abandon(params);
                    }
                    self.state = RaState::AwaitResponse {
                        wt_seen,
                        last_frame: frame,
                    };
                }
                None
            }

            RaState::FirstTry { init_frame } => {
                // cl. 23.5.1.4.5: transmit in the first valid access opportunity
                // provided the IMM condition holds; otherwise fall through to a
                // new access frame.
                let frames_elapsed = (frame_index(dltime) - init_frame).rem_euclid(tetra_ms_types::tdma_time::TIME_INT_WRAP / 4);
                let within_imm = self.is_emergency || params.imm == 15 || frames_elapsed < i32::from(params.imm);

                if ul_slot_valid && within_imm {
                    let s1 = assign.subslot1.is_opportunity_for(self.code);
                    let s2 = assign.subslot2.is_opportunity_for(self.code);
                    let subslot = match (s1, s2) {
                        (true, true) => rng.choose_one_of_two(),
                        (true, false) => Subslot::One,
                        (false, true) => Subslot::Two,
                        (false, false) => return None,
                    };
                    return Some(self.transmit(dltime, subslot));
                }

                // IMM frames have elapsed without transmitting (non-emergency,
                // IMM != 15): switch to choosing a new access frame and process
                // this slot in that mode (cl. 23.5.1.4.5 → .4.6).
                if !within_imm {
                    self.state = RaState::AwaitFrameMarker;
                    return self.begin_frame(dltime, assign, ul_slot_valid, params, rng);
                }
                None
            }

            RaState::AwaitFrameMarker => self.begin_frame(dltime, assign, ul_slot_valid, params, rng),

            RaState::Counting { .. } => {
                // cl. 23.5.1.4.7: continue counting access opportunities toward
                // the chosen subslot.
                if !ul_slot_valid {
                    return None;
                }
                self.count_slot(dltime, assign)
            }
        }
    }

    fn begin_frame(
        &mut self,
        dltime: TdmaTime,
        assign: &SlotAccessAssign,
        ul_slot_valid: bool,
        params: &RandomAccessParams,
        choices: &mut RaChoices,
    ) -> Option<RaAction> {
        if !ul_slot_valid {
            return None;
        }
        let base = self.marker_base(&assign.subslot1)?;
        let length = frame_length(base, params.fl_factor);
        let chosen = choices.choose_subslot_index(length).clamp(1, length);
        self.state = RaState::Counting { chosen, counted: 0 };
        self.count_slot(dltime, assign)
    }

    /// If subslot 1 of `assign` carries a frame marker for the access code,
    /// return its base frame-length (cl. 23.5.1.4.6). The frame marker is always
    /// in subslot 1 of the marker slot (cl. 23.5.1.4.2 point e).
    fn marker_base(&self, subslot1: &SubslotAccess) -> Option<u16> {
        if subslot1.is_frame_marker_for(self.code) {
            subslot1.frame_len.base_subslots()
        } else {
            None
        }
    }

    /// Count the (up to two) access opportunities in this slot toward the chosen
    /// subslot, transmitting when the chosen count is reached (cl. 23.5.1.4.7).
    /// The first counted subslot is the frame-marker subslot (subslot 1 of the
    /// marker slot).
    fn count_slot(&mut self, dltime: TdmaTime, assign: &SlotAccessAssign) -> Option<RaAction> {
        for (idx, sub) in [(Subslot::One, assign.subslot1), (Subslot::Two, assign.subslot2)] {
            if !sub.is_opportunity_for(self.code) {
                continue;
            }
            let RaState::Counting { chosen, counted } = self.state else {
                return None;
            };
            let counted = counted + 1;
            if counted >= chosen {
                return Some(self.transmit(dltime, idx));
            }
            self.state = RaState::Counting { chosen, counted };
        }
        None
    }

    /// Inform the state machine that a MAC-RESOURCE was received on the downlink
    /// (cl. 23.5.1.4.8). `addr_matches` is true when the PDU is addressed to this
    /// MS, and `random_access_success` is the MAC-RESOURCE "random access flag"
    /// acknowledging a successful random access (cl. 21.4.3.1). Returns
    /// `Some(Succeeded)` when the attempt is completed.
    pub fn on_mac_resource(&mut self, addr_matches: bool, random_access_success: bool) -> Option<RaAction> {
        if self.is_active() && addr_matches && random_access_success {
            self.state = RaState::Idle;
            Some(RaAction::Succeeded)
        } else {
            None
        }
    }

    /// Abandon any in-progress attempt without a response (cl. 23.5.1.4.9, e.g.
    /// on TMA-CANCEL). Resets to idle.
    pub fn cancel(&mut self) {
        self.state = RaState::Idle;
        self.tx_count = 0;
    }
}

const _: () = {
    assert!(core::mem::size_of::<MsRandomAccess>() <= 32);
    assert!(core::mem::size_of::<AccessParamStore>() <= 256);
    assert!(core::mem::size_of::<RaAction>() <= 32);
    assert!(core::mem::size_of::<AccessStatus>() <= 8);
};
