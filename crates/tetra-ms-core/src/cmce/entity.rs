use super::{
    downlink::{self, Downlink},
    *,
};
use crate::queue::Queue;

pub const MAX_CALLS: usize = 16;
pub const MAX_ORIGINATIONS: usize = 16;
pub const MAX_ACTIONS: usize = 40;
pub const DEFAULT_BASIC: BasicService = BasicService {
    circuit_mode: 0,
    encrypted: false,
    communication: 1,
    parameter: 0,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallRoute {
    pub main_address: TetraAddress,
    pub handle: u32,
    pub endpoint_id: u32,
    pub link_id: u32,
}

#[derive(Clone, Debug)]
pub struct CmceCall {
    pub protocol: CallState,
    pub basic_service: BasicService,
    pub route: CallRoute,
}
impl core::ops::Deref for CmceCall {
    type Target = CallState;
    fn deref(&self) -> &Self::Target {
        &self.protocol
    }
}
impl core::ops::DerefMut for CmceCall {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.protocol
    }
}
impl CmceCall {
    pub fn create(
        id: u16,
        state: MsCcState,
        basic: BasicService,
        duplex: bool,
        route: CallRoute,
        allowed: bool,
    ) -> Result<Self, CmceError> {
        basic.write(&mut PackedBits::new())?;
        Ok(Self {
            protocol: CallState::new(id, state, kind_from_communication(basic.communication)?, duplex, allowed, 0)?,
            basic_service: basic,
            route,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NoticeKind {
    Setup,
    Proceed,
    Alert,
    SetupConfirm,
    CompleteConfirm,
    TxConfirm,
    TxGranted,
    TxCeased,
    TxWait,
    TxContinue,
    TxInterrupt,
    ReleaseIndication,
    ReleaseConfirm,
    Info,
}

#[derive(Clone, Debug)]
pub struct CmceNotice {
    pub route: CallRoute,
    pub kind: NoticeKind,
    pub id: u16,
    pub fields: Downlink,
    pub sdu: PackedBits,
    pub basic: BasicService,
    pub timeout: u8,
    pub grant: u8,
    pub speaker: Option<u32>,
    pub called_party: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpeechRoute {
    pub id: u16,
    pub sequence: u32,
    pub speaker: Option<u32>,
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // Bounded inline wire payloads, never Box/alloc.
pub enum CmceAction {
    Uplink {
        sdu: PackedBits,
        route: CallRoute,
        stealing: bool,
        repeats: bool,
    },
    Configure {
        id: u16,
        route: CallRoute,
        basic: BasicService,
        state: MsUPlaneState,
    },
    Notify(CmceNotice),
    Speech(SpeechRoute),
    Call {
        change: CallChange,
        id: u16,
        new_id: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CallChange {
    Created,
    Rekeyed,
    Removed,
}

#[derive(Clone, Copy, Debug)]
pub enum CmceEvent<'a> {
    Identity(Option<u32>),
    Downlink {
        sdu: BitView<'a>,
        route: CallRoute,
    },
    Originate {
        called: TetraAddress,
        basic: BasicService,
        duplex: bool,
        request_tx: bool,
        hook: bool,
        external: Option<BitView<'a>>,
    },
    Alert {
        id: u16,
        basic: Option<BasicService>,
        duplex: bool,
    },
    Connect {
        id: u16,
        basic: BasicService,
        duplex: bool,
    },
    Answer {
        id: u16,
        basic: Option<BasicService>,
        duplex: bool,
        complete: bool,
    },
    Demand {
        id: u16,
        priority: u8,
    },
    Cease {
        id: u16,
    },
    Disconnect {
        id: u16,
        cause: u8,
    },
    Release {
        id: u16,
        cause: u8,
    },
    Restore {
        id: u16,
    },
    Dtmf {
        id: u16,
        data: BitView<'a>,
    },
    Traffic {
        owner: Option<u32>,
    },
    Tick,
    Break,
    Reopen,
    Transmitted {
        id: u16,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CmceStatus {
    pub own_issi: Option<u32>,
    pub calls: u16,
    pub originations: u16,
    pub queued_actions: u16,
}

pub struct CmceCore {
    pub own_issi: Option<u32>,
    pub dltime: TdmaTime,
    pub calls: CallTable<CmceCall, MAX_CALLS>,
    pub pending_originations: PendingStack<PendingOrigination<BasicService>, MAX_ORIGINATIONS>,
    actions: Queue<CmceAction, MAX_ACTIONS>,
}
const _: () = {
    assert!(!core::mem::needs_drop::<CmceCore>());
    assert!(!core::mem::needs_drop::<CmceCall>());
};
impl Default for CmceCore {
    fn default() -> Self {
        Self::new()
    }
}
impl CmceCore {
    /// Initialize caller-owned storage without a complete temporary call entity.
    pub fn init_in_place(storage: &mut core::mem::MaybeUninit<Self>) -> &mut Self {
        // SAFETY: MaybeUninit grants exclusive, aligned, sufficiently sized
        // storage; init_at initializes every field before the reference escapes.
        unsafe {
            Self::init_at(storage.as_mut_ptr());
            storage.assume_init_mut()
        }
    }

    /// Initialize caller-owned storage without a complete entity-sized temporary.
    ///
    /// # Safety
    /// `out` must be aligned, writable, exclusively owned storage for `Self`.
    /// This initializes uninitialized storage; no previous value is dropped.
    pub unsafe fn init_at(out: *mut Self) {
        // SAFETY: the caller supplies the complete entity; all fields are written.
        unsafe {
            core::ptr::addr_of_mut!((*out).own_issi).write(None);
            core::ptr::addr_of_mut!((*out).dltime).write(TdmaTime::default());
            CallTable::init_at(core::ptr::addr_of_mut!((*out).calls));
            PendingStack::init_at(core::ptr::addr_of_mut!((*out).pending_originations));
            Queue::init_at(core::ptr::addr_of_mut!((*out).actions));
        }
    }
    pub fn new() -> Self {
        Self {
            own_issi: None,
            dltime: TdmaTime::default(),
            calls: CallTable::new(),
            pending_originations: PendingStack::new(),
            actions: Queue::new(),
        }
    }
    pub fn status(&self) -> CmceStatus {
        CmceStatus {
            own_issi: self.own_issi,
            calls: self.calls.len() as u16,
            originations: self.pending_originations.len() as u16,
            queued_actions: self.actions.len(),
        }
    }
    /// Clear identity, calls, pending originations, time and queued actions.
    pub fn reset(&mut self) {
        self.own_issi = None;
        self.dltime = TdmaTime::default();
        self.calls = CallTable::new();
        self.pending_originations = PendingStack::new();
        self.actions.clear();
    }
    pub fn pop_action(&mut self) -> Option<CmceAction> {
        self.actions.pop()
    }
    fn reserve(&self, count: usize) -> Result<(), CmceError> {
        if usize::from(self.actions.free()) < count {
            Err(CmceError::OutputFull)
        } else {
            Ok(())
        }
    }
    fn uplink(&mut self, sdu: PackedBits, mut route: CallRoute, stealing: bool) {
        route.main_address = signalling_address(route.main_address, self.own_issi);
        self.actions.push(CmceAction::Uplink {
            sdu,
            route,
            stealing,
            repeats: stealing,
        });
    }
    pub fn configure(&mut self, id: u16, state: MsUPlaneState) -> Result<(), CmceError> {
        let call = self.calls.get(&id).ok_or(CmceError::UnknownCall)?;
        if self.calls.values().any(|other| other.blocks_uplane_of(id, call.kind)) {
            return Ok(());
        }
        self.reserve(1)?;
        let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
        call.configure(state);
        self.actions.push(CmceAction::Configure {
            id,
            route: call.route,
            basic: call.basic_service,
            state,
        });
        Ok(())
    }
    pub fn apply_grant(&mut self, id: u16, grant: u8, speaker: Option<u32>) -> Result<(), CmceError> {
        self.reserve(1)?;
        let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
        if let Some(state) = call.protocol.apply_grant(self.own_issi, grant, speaker)? {
            self.configure(id, state)?;
        } else {
            self.revoke_stale_tx(id)?;
        }
        Ok(())
    }
    fn revoke_stale_tx(&mut self, id: u16) -> Result<(), CmceError> {
        let call = self.calls.get(&id).ok_or(CmceError::UnknownCall)?;
        if call.tx_grant_state != MsTxGrantState::GrantedSelf
            && let Some(previous) = call.last_uplane.filter(|state| state.tx_grant)
        {
            self.configure(
                id,
                MsUPlaneState {
                    tx_grant: false,
                    ..previous
                },
            )?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn notify(&mut self, kind: NoticeKind, id: u16, fields: Downlink, sdu: &PackedBits, grant: u8, speaker: Option<u32>, route: CallRoute) {
        let basic = self.calls.get(&id).map_or(DEFAULT_BASIC, |call| call.basic_service);
        let timeout = self.calls.get(&id).map_or(0, |call| call.timers.call_timeout);
        let called_party = self.calls.get(&id).map_or(0, |call| call.route.main_address.ssi);
        self.actions.push(CmceAction::Notify(CmceNotice {
            route,
            kind,
            id,
            fields,
            sdu: sdu.clone(),
            basic,
            timeout,
            grant,
            speaker,
            called_party,
        }));
    }
    fn floor_route(&self, id: u16) -> Result<(CallRoute, bool), CmceError> {
        let call = self.calls.get(&id).ok_or(CmceError::UnknownCall)?;
        let (main_address, link_id, stealing) =
            super::floor_route(call.route.main_address, self.own_issi, call.route.link_id, call.last_uplane);
        Ok((
            CallRoute {
                main_address,
                link_id,
                ..call.route
            },
            stealing,
        ))
    }
    /// Errors never consume an event or mutate protocol state. Conservative
    /// output reservation makes retrying after draining actions deterministic.
    pub fn push_event(&mut self, now: TdmaTime, event: CmceEvent<'_>) -> Result<(), CmceError> {
        let count = match event {
            CmceEvent::Tick => 2 * self.calls.len(),
            CmceEvent::Break => self.calls.len(),
            CmceEvent::Reopen => 2 * self.calls.len(),
            CmceEvent::Downlink { .. } => 4,
            _ => 2,
        };
        self.reserve(count)?;
        let old_time = self.dltime;
        self.dltime = now;
        let result = self.dispatch(event);
        if result.is_err() {
            self.dltime = old_time;
        }
        result
    }
    fn dispatch(&mut self, event: CmceEvent<'_>) -> Result<(), CmceError> {
        match event {
            CmceEvent::Identity(own) => {
                if own.is_some_and(|ssi| ssi == 0 || ssi > 0xff_ffff) {
                    return Err(CmceError::InvalidValue);
                }
                self.own_issi = own;
            }
            CmceEvent::Downlink { sdu, route } => {
                let fields = downlink::decode(sdu)?;
                let sdu = PackedBits::from_view(sdu)?;
                self.downlink(fields, &sdu, route)?;
            }
            CmceEvent::Originate {
                called,
                basic,
                duplex,
                request_tx,
                hook,
                external,
            } => {
                let own = self.own_issi.ok_or(CmceError::NotAllowed)?;
                if own == 0 || own > 0xff_ffff {
                    return Err(CmceError::InvalidValue);
                }
                if self.pending_originations.is_full() {
                    return Err(CmceError::Capacity);
                }
                let sdu = encode_uplink(Uplink::Setup {
                    called: called.ssi,
                    basic,
                    hook,
                    duplex,
                    request_tx,
                    external,
                })?;
                self.pending_originations.push(PendingOrigination {
                    called_party: called,
                    basic_service: basic,
                    simplex_duplex_selection: duplex,
                })?;
                self.uplink(
                    sdu,
                    CallRoute {
                        main_address: TetraAddress::new(own, SsiType::Issi),
                        handle: 0,
                        endpoint_id: 0,
                        link_id: 0,
                    },
                    false,
                );
            }
            CmceEvent::Alert { id, basic, duplex } => self.answer(id, basic, duplex, true)?,
            CmceEvent::Connect { id, basic, duplex } => self.answer(id, Some(basic), duplex, false)?,
            CmceEvent::Answer {
                id,
                basic,
                duplex,
                complete,
            } => {
                let alert = !complete && self.calls.get(&id).ok_or(CmceError::UnknownCall)?.needs_alert();
                self.answer(id, basic, duplex, alert)?;
            }
            CmceEvent::Demand { id, priority } => {
                let (route, stealing) = self.floor_route(id)?;
                let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
                let sdu = encode_uplink(Uplink::TxDemand {
                    id,
                    priority,
                    encrypted: call.basic_service.encrypted,
                })?;
                call.request_tx(priority)?;
                self.uplink(sdu, route, stealing);
            }
            CmceEvent::Cease { id } => {
                let (route, stealing) = self.floor_route(id)?;
                let sdu = encode_uplink(Uplink::TxCeased { id })?;
                let state = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?.cease(None);
                self.uplink(sdu, route, stealing);
                self.configure(id, state)?;
            }
            CmceEvent::Disconnect { id, cause } => self.clear(id, cause, false)?,
            CmceEvent::Release { id, cause } => self.clear(id, cause, true)?,
            CmceEvent::Restore { id } => {
                let call = self.calls.get(&id).ok_or(CmceError::UnknownCall)?;
                let sdu = encode_uplink(Uplink::Restore {
                    id,
                    other_party: call.route.main_address.ssi,
                    request_tx: call.restoration_requests_tx(),
                    basic: call.basic_service,
                })?;
                self.uplink(sdu, call.route, false);
            }
            CmceEvent::Dtmf { id, data } => {
                let call = self.calls.get(&id).ok_or(CmceError::UnknownCall)?;
                if !call.allows_dtmf() {
                    return Err(CmceError::NotAllowed);
                }
                let decoded = decode_dtmf(data)?;
                if decoded.kind > 1 {
                    return Err(CmceError::NotAllowed);
                }
                let sdu = encode_uplink(Uplink::Info { id, dtmf: data })?;
                let (route, stealing) = self.floor_route(id)?;
                self.uplink(sdu, route, stealing);
            }
            CmceEvent::Traffic { owner } => {
                if let Some(speech) = self.receive_traffic(owner) {
                    self.actions.push(CmceAction::Speech(speech));
                }
            }
            CmceEvent::Transmitted { id } => {
                self.transmitted(id)?;
            }
            CmceEvent::Tick => {
                let mut ids = [None; MAX_CALLS];
                for (slot, (id, call)) in ids.iter_mut().zip(self.calls.iter()) {
                    *slot = call.timer_expiry_cause(self.dltime).map(|cause| (*id, cause));
                }
                for (id, cause) in ids.into_iter().flatten() {
                    self.clear(id, cause, false)?;
                }
            }
            CmceEvent::Break | CmceEvent::Reopen => {
                let reopen = matches!(event, CmceEvent::Reopen);
                let mut ids = [None; MAX_CALLS];
                for (slot, id) in ids.iter_mut().zip(self.calls.keys()) {
                    *slot = Some(*id);
                }
                for id in ids.into_iter().flatten() {
                    let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
                    let state = if reopen { call.clear(true, 19) } else { call.broken() };
                    self.configure(id, state)?;
                    if reopen {
                        self.calls.remove(&id);
                        self.actions.push(CmceAction::Call {
                            change: CallChange::Removed,
                            id,
                            new_id: 0,
                        });
                    }
                }
            }
        }
        Ok(())
    }
    fn answer(&mut self, id: u16, basic: Option<BasicService>, duplex: bool, alert: bool) -> Result<(), CmceError> {
        let call = self.calls.get(&id).ok_or(CmceError::UnknownCall)?;
        let basic = basic.unwrap_or(call.basic_service);
        let sdu = encode_uplink(if alert {
            Uplink::Alert { id, duplex, basic }
        } else {
            Uplink::Connect {
                id,
                hook: call.hook_on_off,
                duplex,
                basic,
            }
        })?;
        let route = call.route;
        if !alert {
            let timeout = call.timers.setup_timeout.unwrap_or(0);
            self.calls
                .get_mut(&id)
                .ok_or(CmceError::UnknownCall)?
                .start_setup_timer(self.dltime, timeout, timeout)?;
        }
        self.uplink(sdu, route, false);
        Ok(())
    }
    fn clear(&mut self, id: u16, cause: u8, release: bool) -> Result<(), CmceError> {
        let sdu = encode_uplink(if release {
            Uplink::Release { id, cause }
        } else {
            Uplink::Disconnect { id, cause }
        })?;
        let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
        let state = call.clear(release, cause);
        call.tx_grant_state = MsTxGrantState::None;
        call.pending_tx_request = false;
        // Clearing consumes the expired setup/call timer. There is no locally
        // invented retransmission timer: subsequent ticks must not resend.
        call.timers.setup_phase_deadline = None;
        call.timers.call_deadline = None;
        let route = call.route;
        self.uplink(sdu, route, false);
        self.configure(id, state)?;
        Ok(())
    }
    fn downlink(&mut self, p: Downlink, sdu: &PackedBits, route: CallRoute) -> Result<(), CmceError> {
        let id = p.id;
        match p.kind {
            7 => {
                let basic = p.basic.ok_or(CmceError::InvalidValue)?;
                let kind = kind_from_communication(basic.communication)?;
                if ignore_group_setup(kind, self.own_issi, p.speaker) {
                    return Ok(());
                }
                if kind != MsCallKind::Individual && self.calls.contains_key(&id) {
                    let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
                    let previous = call.current_speaker_ssi;
                    call.route = route;
                    if !call.group_rebroadcast(p.permission_denied) {
                        return Ok(());
                    }
                    self.apply_grant(id, p.grant, p.speaker)?;
                    let speaker = self.calls.get(&id).and_then(|call| call.current_speaker_ssi);
                    if speaker != previous {
                        self.notify(
                            NoticeKind::TxGranted,
                            id,
                            Downlink { extension: None, ..p },
                            sdu,
                            resolve_floor_grant(self.own_issi, p.grant, p.speaker)?,
                            speaker,
                            route,
                        );
                    }
                    return Ok(());
                }
                let mut call = CmceCall::create(id, MsCcState::MtCallSetup, basic, p.duplex, route, !p.permission_denied)?;
                let timeout = p.timeout.unwrap_or(0);
                call.incoming(p.speaker, p.hook, self.dltime, timeout, timeout)?;
                self.calls.insert(id, call)?;
                self.actions.push(CmceAction::Call {
                    change: CallChange::Created,
                    id,
                    new_id: 0,
                });
                self.notify(NoticeKind::Setup, id, p, sdu, p.grant, p.speaker, route);
                if kind != MsCallKind::Individual {
                    let grant = resolve_floor_grant(self.own_issi, p.grant, p.speaker)?;
                    self.apply_grant(id, p.grant, p.speaker)?;
                    if grant == 3 {
                        self.notify(NoticeKind::TxGranted, id, p, sdu, grant, p.speaker, route);
                    }
                }
            }
            1 | 2 => {
                if self.calls.is_full() && !self.calls.contains_key(&id) {
                    return Err(CmceError::Capacity);
                }
                if !self.calls.contains_key(&id) {
                    let pending = self.pending_originations.iter().last();
                    let (basic, duplex) = originating_service(p.basic, pending, p.duplex, DEFAULT_BASIC);
                    self.calls.insert(
                        id,
                        CmceCall::create(
                            id,
                            MsCcState::MoCallSetup,
                            basic,
                            duplex,
                            route,
                            if p.kind == 1 { true } else { !p.permission_denied },
                        )?,
                    )?;
                    self.pending_originations.pop();
                    self.actions.push(CmceAction::Call {
                        change: CallChange::Created,
                        id,
                        new_id: 0,
                    });
                }
                let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
                call.route = route;
                if p.kind == 1 {
                    let timeout = p.setup_timeout.unwrap_or(0);
                    call.proceed(self.dltime, timeout, timeout)?;
                    self.notify(NoticeKind::Proceed, id, p, sdu, p.grant, None, route);
                } else {
                    let timeout = p.timeout.unwrap_or(0);
                    call.connect(self.dltime, timeout, timeout, p.permission_denied, Some(p.duplex))?;
                    self.apply_grant(id, p.grant, None)?;
                    self.notify(NoticeKind::SetupConfirm, id, p, sdu, p.grant, None, route);
                }
            }
            0 => {
                if let Some(call) = self.calls.get_mut(&id) {
                    call.route = route;
                    if let Some(basic) = p.basic {
                        call.basic_service = basic;
                    }
                    let timeout = p.setup_timeout.unwrap_or(0);
                    call.start_setup_timer(self.dltime, timeout, timeout)?;
                }
                self.notify(NoticeKind::Alert, id, p, sdu, p.grant, None, route);
            }
            3 => {
                if let Some(call) = self.calls.get_mut(&id) {
                    call.route = route;
                    let timeout = p.timeout.unwrap_or(0);
                    call.connect(self.dltime, timeout, timeout, p.permission_denied, None)?;
                    self.apply_grant(id, p.grant, None)?;
                    self.notify(NoticeKind::CompleteConfirm, id, p, sdu, p.grant, None, route);
                }
            }
            11 => {
                if let Some(call) = self.calls.get_mut(&id) {
                    call.route = route;
                    let outcome = call.granted_event(self.own_issi, p.grant, p.speaker, p.permission_denied)?;
                    if outcome.ignored {
                        return Ok(());
                    }
                    if let Some(state) = outcome.configure {
                        self.configure(id, state)?;
                    } else {
                        self.revoke_stale_tx(id)?;
                    }
                    let speaker = p.speaker.or_else(|| self.calls.get(&id).and_then(|call| call.current_speaker_ssi));
                    self.notify(
                        if outcome.confirm_request {
                            NoticeKind::TxConfirm
                        } else {
                            NoticeKind::TxGranted
                        },
                        id,
                        p,
                        sdu,
                        outcome.grant,
                        speaker,
                        route,
                    );
                }
            }
            9 | 10 | 12 | 13 => {
                if let Some(call) = self.calls.get_mut(&id) {
                    call.route = route;
                    let state = match p.kind {
                        9 => Some(call.cease(Some(p.permission_denied))),
                        10 => call.continue_tx(p.permission_denied, p.resume),
                        12 => Some(call.wait(p.permission_denied)),
                        _ => call.interrupted_event(self.own_issi, p.grant, p.speaker, p.permission_denied)?,
                    };
                    if let Some(state) = state {
                        self.configure(id, state)?;
                    }
                }
                let kind = match p.kind {
                    9 => NoticeKind::TxCeased,
                    10 => NoticeKind::TxContinue,
                    12 => NoticeKind::TxWait,
                    _ => NoticeKind::TxInterrupt,
                };
                self.notify(kind, id, p, sdu, p.grant, p.speaker, route);
            }
            4 => {
                self.notify(NoticeKind::ReleaseIndication, id, p, sdu, p.grant, None, route);
                if let Some(call) = self.calls.get_mut(&id) {
                    call.route = route;
                    call.clear(false, p.cause);
                    self.clear(id, p.cause, true)?;
                }
            }
            6 => {
                let local = self.calls.get(&id).is_some_and(|call| call.locally_clearing());
                if let Some(call) = self.calls.get_mut(&id) {
                    call.route = route;
                    let state = call.clear(true, p.cause);
                    self.configure(id, state)?;
                }
                self.notify(
                    if local {
                        NoticeKind::ReleaseConfirm
                    } else {
                        NoticeKind::ReleaseIndication
                    },
                    id,
                    p,
                    sdu,
                    p.grant,
                    None,
                    route,
                );
                self.calls.remove(&id);
                self.actions.push(CmceAction::Call {
                    change: CallChange::Removed,
                    id,
                    new_id: 0,
                });
            }
            5 | 14 => {
                let mut key = id;
                if p.kind == 5
                    && let Some(call) = self.calls.get_mut(&id)
                {
                    call.route = route;
                    call.info(self.dltime, p.setup_timeout.map(|v| (v, v)), p.timeout.map(|v| (v, v)), p.reset)?;
                }
                if let Some(new_id) = p.new_id
                    && self.calls.contains_key(&id)
                {
                    self.calls.remap(id, new_id)?;
                    if id != new_id {
                        self.actions.push(CmceAction::Call {
                            change: CallChange::Rekeyed,
                            id,
                            new_id,
                        });
                    }
                    key = new_id;
                    if let Some(call) = self.calls.get_mut(&key) {
                        call.call_identifier = key;
                    }
                }
                if p.kind == 5 {
                    // Unknown calls still deliver the original indication; only
                    // an actually tracked call can adopt a new identifier.
                    self.notify(NoticeKind::Info, key, p, sdu, p.grant, None, route);
                } else if let Some(call) = self.calls.get_mut(&key) {
                    call.route = route;
                    call.restore(self.dltime, p.timeout.map(|v| (v, v)), p.reset)?;
                    self.apply_grant(key, p.grant, None)?;
                }
            }
            _ => return Err(CmceError::Unsupported),
        }
        Ok(())
    }
    pub fn receive_traffic(&mut self, owner: Option<u32>) -> Option<SpeechRoute> {
        let mut target = owner.and_then(|owner| {
            self.calls
                .values()
                .find(|call| call.route.main_address.ssi == owner && call.last_uplane.is_some_and(|u| u.switch_u_plane))
                .map(|call| call.call_identifier)
        });
        if target.is_none() {
            let mut active = self
                .calls
                .values()
                .filter(|call| call.last_uplane.is_some_and(|u| u.switch_u_plane));
            if let (Some(call), None) = (active.next(), active.next()) {
                target = Some(call.call_identifier);
            }
        }
        let call = self.calls.get_mut(&target?)?;
        if !call.accepts_downlink() {
            return None;
        }
        call.rx_speech_frames = call.rx_speech_frames.saturating_add(1);
        Some(SpeechRoute {
            id: call.call_identifier,
            sequence: call.rx_speech_frames,
            speaker: call.current_speaker_ssi,
        })
    }
    pub fn transmit_target(&self) -> Option<u16> {
        self.calls
            .values()
            .find(|call| call.accepts_uplink())
            .map(|call| call.call_identifier)
    }
    pub fn transmitted(&mut self, id: u16) -> Result<u32, CmceError> {
        let call = self.calls.get_mut(&id).ok_or(CmceError::UnknownCall)?;
        if !call.accepts_uplink() {
            return Err(CmceError::NotAllowed);
        }
        call.tx_speech_frames = call.tx_speech_frames.saturating_add(1);
        Ok(call.tx_speech_frames)
    }
}
