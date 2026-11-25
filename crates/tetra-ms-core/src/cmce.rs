//! Caller-owned CMCE call/floor state, timers and uplink wire behavior.
//!
//! The generic timer/cause parameters are opaque application vocabulary. Wire
//! values are validated separately; the state machine never calls platform code.
//!
//! `CmceCore` owns the bounded entity, downlink decoding, call orchestration and
//! traffic attribution. Desktop integration translates its actions to SAPs.
use tetra_ms_types::{BitError, BitView, PackedBits, SsiType, TdmaTime, TetraAddress};

pub mod downlink;
mod entity;
pub use entity::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MsCcState {
    Idle,
    MoCallSetup,
    MtCallSetup,
    CallActive,
    Wait,
    Restore,
    Disconnect,
    Release,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MsCallKind {
    Individual,
    Group,
    AcknowledgedGroup,
    Broadcast,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MsTxGrantState {
    None,
    GrantedSelf,
    RequestQueued,
    GrantedOther,
    Waiting,
    Interrupted,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsUPlaneState {
    pub switch_u_plane: bool,
    pub tx_grant: bool,
    pub simplex_duplex: bool,
}

#[derive(Clone, Debug)]
pub struct MsCallTimers<T, S> {
    pub setup_phase_deadline: Option<TdmaTime>,
    pub call_deadline: Option<TdmaTime>,
    pub setup_timeout: Option<S>,
    pub call_timeout: T,
}

#[derive(Clone, Debug)]
pub struct CallState<T = u8, S = u8, C = u8> {
    pub call_identifier: u16,
    pub state: MsCcState,
    pub kind: MsCallKind,
    pub current_speaker_ssi: Option<u32>,
    pub tx_grant_state: MsTxGrantState,
    pub transmission_request_allowed: bool,
    pub timers: MsCallTimers<T, S>,
    pub disconnect_cause: Option<C>,
    pub last_uplane: Option<MsUPlaneState>,
    pub rx_speech_frames: u32,
    pub tx_speech_frames: u32,
    pub simplex_duplex_selection: bool,
    pub hook_on_off: bool,
    pub pending_tx_request: bool,
    pub uplane_before_wait: Option<MsUPlaneState>,
    call_timeout_raw: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CmceError {
    InvalidValue,
    UnknownCall,
    Capacity,
    NotAllowed,
    Bits(BitError),
    Unsupported,
    OutputFull,
}
impl From<BitError> for CmceError {
    fn from(value: BitError) -> Self {
        Self::Bits(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallStatus {
    pub id: u16,
    pub state: MsCcState,
    pub kind: MsCallKind,
    pub grant: MsTxGrantState,
    pub speaker: Option<u32>,
    pub request_allowed: bool,
    pub request_pending: bool,
    pub uplane: Option<MsUPlaneState>,
    pub setup_deadline: Option<TdmaTime>,
    pub call_deadline: Option<TdmaTime>,
}

#[derive(Clone, Copy, Debug)]
pub enum CallEvent {
    Proceed {
        timeout: u8,
    },
    Connect {
        timeout: u8,
        permission_denied: bool,
        duplex: Option<bool>,
    },
    Grant {
        own: Option<u32>,
        grant: u8,
        speaker: Option<u32>,
    },
    Demand {
        priority: u8,
    },
    Ceased {
        permission_denied: Option<bool>,
    },
    Wait {
        permission_denied: bool,
    },
    Continue {
        permission_denied: bool,
        resume: bool,
    },
    Interrupt,
    Clear {
        release: bool,
        cause: u8,
    },
    Break,
    Info {
        setup: Option<u8>,
        timeout: Option<u8>,
        reset: bool,
    },
    Restore {
        timeout: Option<u8>,
        reset: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallAction {
    None,
    Configure(MsUPlaneState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FloorOutcome {
    pub grant: u8,
    pub configure: Option<MsUPlaneState>,
    pub confirm_request: bool,
    pub ignored: bool,
}

impl CallState {
    /// Validated fixed-width event surface for portable coordinators. `Configure`
    /// is a requested action: apply cross-call arbitration before `configure`.
    pub fn push_event(&mut self, now: TdmaTime, event: CallEvent) -> Result<CallAction, CmceError> {
        match event {
            CallEvent::Proceed { timeout } if timeout > 7 => return Err(CmceError::InvalidValue),
            CallEvent::Connect { timeout, .. } if timeout > 15 => return Err(CmceError::InvalidValue),
            CallEvent::Info { setup, timeout, .. } if setup.is_some_and(|v| v > 7) || timeout.is_some_and(|v| v > 15) => {
                return Err(CmceError::InvalidValue);
            }
            CallEvent::Restore { timeout, .. } if timeout.is_some_and(|v| v > 15) => return Err(CmceError::InvalidValue),
            CallEvent::Clear { cause, .. } if cause > 23 => return Err(CmceError::InvalidValue),
            _ => {}
        }
        let uplane = match event {
            CallEvent::Proceed { timeout } => {
                self.proceed(now, timeout, timeout)?;
                None
            }
            CallEvent::Connect {
                timeout,
                permission_denied,
                duplex,
            } => {
                self.connect(now, timeout, timeout, permission_denied, duplex)?;
                None
            }
            CallEvent::Grant { own, grant, speaker } => self.apply_grant(own, grant, speaker)?,
            CallEvent::Demand { priority } => {
                self.request_tx(priority)?;
                None
            }
            CallEvent::Ceased { permission_denied } => Some(self.cease(permission_denied)),
            CallEvent::Wait { permission_denied } => Some(self.wait(permission_denied)),
            CallEvent::Continue { permission_denied, resume } => self.continue_tx(permission_denied, resume),
            CallEvent::Interrupt => Some(self.interrupt()),
            CallEvent::Clear { release, cause } => Some(self.clear(release, cause)),
            CallEvent::Break => Some(self.broken()),
            CallEvent::Info { setup, timeout, reset } => {
                self.info(now, setup.map(|v| (v, v)), timeout.map(|v| (v, v)), reset)?;
                None
            }
            CallEvent::Restore { timeout, reset } => {
                self.restore(now, timeout.map(|v| (v, v)), reset)?;
                None
            }
        };
        Ok(uplane.map_or(CallAction::None, CallAction::Configure))
    }
}

impl<T: Copy, S: Copy, C: Copy> CallState<T, S, C> {
    pub fn new(id: u16, state: MsCcState, kind: MsCallKind, duplex: bool, allowed: bool, infinite: T) -> Result<Self, CmceError> {
        if id > 0x3fff {
            return Err(CmceError::InvalidValue);
        }
        Ok(Self {
            call_identifier: id,
            state,
            kind,
            current_speaker_ssi: None,
            tx_grant_state: MsTxGrantState::None,
            transmission_request_allowed: allowed,
            timers: MsCallTimers {
                setup_phase_deadline: None,
                call_deadline: None,
                setup_timeout: None,
                call_timeout: infinite,
            },
            disconnect_cause: None,
            last_uplane: None,
            rx_speech_frames: 0,
            tx_speech_frames: 0,
            simplex_duplex_selection: duplex,
            hook_on_off: false,
            pending_tx_request: false,
            uplane_before_wait: None,
            call_timeout_raw: 0,
        })
    }

    pub fn start_setup_timer(&mut self, now: TdmaTime, value: S, wire: u8) -> Result<(), CmceError> {
        if wire > 7 {
            return Err(CmceError::InvalidValue);
        }
        self.timers.setup_timeout = Some(value);
        self.timers.setup_phase_deadline = setup_timeout_to_timeslots(wire).map(|slots| now.add_timeslots(slots));
        Ok(())
    }
    pub fn start_call_timer(&mut self, now: TdmaTime, value: T, wire: u8) -> Result<(), CmceError> {
        if wire > 15 {
            return Err(CmceError::InvalidValue);
        }
        self.timers.call_timeout = value;
        self.call_timeout_raw = wire;
        self.timers.call_deadline = call_timeout_to_timeslots(wire).map(|slots| now.add_timeslots(slots));
        Ok(())
    }
    pub fn expired(&self, now: TdmaTime) -> bool {
        self.timers.setup_phase_deadline.is_some_and(|deadline| deadline.age(now) >= 0)
            || self.timers.call_deadline.is_some_and(|deadline| deadline.age(now) >= 0)
    }
    pub fn timer_expiry_cause(&self, now: TdmaTime) -> Option<u8> {
        self.expired(now).then_some(13)
    }
    pub fn proceed(&mut self, now: TdmaTime, timeout: S, wire: u8) -> Result<(), CmceError> {
        self.start_setup_timer(now, timeout, wire)?;
        self.state = MsCcState::MoCallSetup;
        Ok(())
    }
    pub fn connect(&mut self, now: TdmaTime, timeout: T, wire: u8, permission_denied: bool, duplex: Option<bool>) -> Result<(), CmceError> {
        self.start_call_timer(now, timeout, wire)?;
        self.state = MsCcState::CallActive;
        self.transmission_request_allowed = !permission_denied;
        if let Some(duplex) = duplex {
            self.simplex_duplex_selection = duplex;
        }
        self.timers.setup_phase_deadline = None;
        Ok(())
    }
    pub fn group_rebroadcast(&mut self, permission_denied: bool) -> bool {
        self.state = MsCcState::CallActive;
        self.transmission_request_allowed = !permission_denied;
        self.tx_grant_state != MsTxGrantState::GrantedSelf
    }
    pub fn incoming(&mut self, speaker: Option<u32>, hook: bool, now: TdmaTime, timeout: T, wire: u8) -> Result<(), CmceError> {
        if speaker.is_some_and(|ssi| ssi > 0xff_ffff) {
            return Err(CmceError::InvalidValue);
        }
        self.start_call_timer(now, timeout, wire)?;
        self.state = if self.kind == MsCallKind::Individual {
            MsCcState::MtCallSetup
        } else {
            MsCcState::CallActive
        };
        self.current_speaker_ssi = speaker;
        self.hook_on_off = hook;
        Ok(())
    }
    pub fn floor_update(&mut self, permission_denied: bool, speaker: Option<u32>) -> Result<(), CmceError> {
        if speaker.is_some_and(|ssi| ssi > 0xff_ffff) {
            return Err(CmceError::InvalidValue);
        }
        self.state = MsCcState::CallActive;
        self.transmission_request_allowed = !permission_denied;
        self.current_speaker_ssi = speaker;
        Ok(())
    }
    /// Returns the requested U-plane configuration. Arbitration is performed
    /// across all calls before committing it with `configure`.
    pub fn apply_grant(&mut self, own: Option<u32>, grant: u8, speaker: Option<u32>) -> Result<Option<MsUPlaneState>, CmceError> {
        let grant = resolve_floor_grant(own, grant, speaker)?;
        self.current_speaker_ssi = speaker.or(self.current_speaker_ssi);
        match grant {
            0 => {
                self.tx_grant_state = MsTxGrantState::GrantedSelf;
                self.pending_tx_request = false;
                Ok(Some(self.uplane(true, true)))
            }
            1 => {
                self.tx_grant_state = MsTxGrantState::None;
                self.pending_tx_request = false;
                Ok(None)
            }
            2 => {
                self.tx_grant_state = MsTxGrantState::RequestQueued;
                self.pending_tx_request = true;
                Ok(None)
            }
            3 => {
                self.tx_grant_state = MsTxGrantState::GrantedOther;
                Ok(Some(self.uplane(true, false)))
            }
            _ => Err(CmceError::InvalidValue),
        }
    }
    pub fn granted_event(
        &mut self,
        own: Option<u32>,
        grant: u8,
        speaker: Option<u32>,
        permission_denied: bool,
    ) -> Result<FloorOutcome, CmceError> {
        let resolved = resolve_floor_grant(own, grant, speaker)?;
        let pending_before = self.pending_tx_request;
        self.floor_update(permission_denied, speaker)?;
        if grant == 3 && speaker == own {
            return Ok(FloorOutcome {
                grant: resolved,
                configure: None,
                confirm_request: false,
                ignored: true,
            });
        }
        let configure = self.apply_grant(own, resolved, speaker)?;
        Ok(FloorOutcome {
            grant: resolved,
            configure,
            confirm_request: pending_before && resolved == 0,
            ignored: false,
        })
    }
    pub fn interrupted_event(
        &mut self,
        own: Option<u32>,
        grant: u8,
        speaker: Option<u32>,
        permission_denied: bool,
    ) -> Result<Option<MsUPlaneState>, CmceError> {
        resolve_floor_grant(own, grant, speaker)?;
        self.floor_update(permission_denied, speaker)?;
        if grant == 3 {
            self.apply_grant(own, grant, speaker)
        } else {
            Ok(Some(self.interrupt()))
        }
    }
    pub fn configure(&mut self, state: MsUPlaneState) {
        self.last_uplane = Some(state);
    }
    pub fn uplane(&self, on: bool, tx: bool) -> MsUPlaneState {
        MsUPlaneState {
            switch_u_plane: on,
            tx_grant: tx,
            simplex_duplex: self.simplex_duplex_selection,
        }
    }
    pub fn cease(&mut self, permission_denied: Option<bool>) -> MsUPlaneState {
        self.tx_grant_state = MsTxGrantState::None;
        self.pending_tx_request = false;
        if let Some(denied) = permission_denied {
            self.state = MsCcState::CallActive;
            self.current_speaker_ssi = None;
            self.transmission_request_allowed = !denied;
        }
        self.uplane(false, false)
    }
    pub fn wait(&mut self, permission_denied: bool) -> MsUPlaneState {
        self.state = MsCcState::Wait;
        self.transmission_request_allowed = !permission_denied;
        self.tx_grant_state = MsTxGrantState::Waiting;
        self.uplane_before_wait = self.last_uplane.filter(|u| u.switch_u_plane);
        self.uplane(false, false)
    }
    pub fn continue_tx(&mut self, permission_denied: bool, do_continue: bool) -> Option<MsUPlaneState> {
        self.state = MsCcState::CallActive;
        self.transmission_request_allowed = !permission_denied;
        let restore = if do_continue { self.uplane_before_wait.take() } else { None };
        self.tx_grant_state = match restore {
            Some(u) if u.tx_grant => MsTxGrantState::GrantedSelf,
            Some(_) => MsTxGrantState::GrantedOther,
            None => MsTxGrantState::None,
        };
        restore
    }
    pub fn interrupt(&mut self) -> MsUPlaneState {
        self.tx_grant_state = MsTxGrantState::Interrupted;
        self.uplane(false, false)
    }
    pub fn request_tx(&mut self, priority: u8) -> Result<(), CmceError> {
        if priority > 3 {
            return Err(CmceError::InvalidValue);
        }
        if !self.transmission_request_allowed {
            return Err(CmceError::NotAllowed);
        }
        self.pending_tx_request = true;
        Ok(())
    }
    pub fn clear(&mut self, release: bool, cause: C) -> MsUPlaneState {
        self.state = if release { MsCcState::Release } else { MsCcState::Disconnect };
        self.disconnect_cause = Some(cause);
        self.uplane(false, false)
    }
    pub fn broken(&mut self) -> MsUPlaneState {
        self.state = MsCcState::Restore;
        if self.tx_grant_state == MsTxGrantState::GrantedSelf {
            self.tx_grant_state = MsTxGrantState::None;
            self.pending_tx_request = false;
        }
        self.uplane(false, false)
    }
    pub fn info(&mut self, now: TdmaTime, setup: Option<(S, u8)>, timeout: Option<(T, u8)>, reset: bool) -> Result<(), CmceError> {
        if setup.is_some_and(|(_, raw)| raw > 7) || timeout.is_some_and(|(_, raw)| raw > 15) {
            return Err(CmceError::InvalidValue);
        }
        if let Some((value, raw)) = setup {
            self.start_setup_timer(now, value, raw)?;
        }
        if reset {
            if let Some((value, raw)) = timeout {
                self.start_call_timer(now, value, raw)?;
            } else if self.call_timeout_raw != 0 {
                self.start_call_timer(now, self.timers.call_timeout, self.call_timeout_raw)?;
            }
        } else if let Some((value, raw)) = timeout {
            self.timers.call_timeout = value;
            self.call_timeout_raw = raw;
        }
        Ok(())
    }
    pub fn restore(&mut self, now: TdmaTime, timeout: Option<(T, u8)>, reset: bool) -> Result<(), CmceError> {
        if timeout.is_some_and(|(_, raw)| raw > 15) {
            return Err(CmceError::InvalidValue);
        }
        if reset {
            self.info(now, None, timeout, true)?;
        }
        self.state = MsCcState::CallActive;
        Ok(())
    }
    pub fn accepts_downlink(&self) -> bool {
        self.last_uplane.is_some_and(|u| u.switch_u_plane)
            && (self.simplex_duplex_selection || self.tx_grant_state != MsTxGrantState::GrantedSelf)
    }
    pub fn accepts_uplink(&self) -> bool {
        self.tx_grant_state == MsTxGrantState::GrantedSelf && self.last_uplane.is_some_and(|u| u.switch_u_plane)
    }
    pub fn status(&self) -> CallStatus {
        CallStatus {
            id: self.call_identifier,
            state: self.state,
            kind: self.kind,
            grant: self.tx_grant_state,
            speaker: self.current_speaker_ssi,
            request_allowed: self.transmission_request_allowed,
            request_pending: self.pending_tx_request,
            uplane: self.last_uplane,
            setup_deadline: self.timers.setup_phase_deadline,
            call_deadline: self.timers.call_deadline,
        }
    }
    pub fn blocks_uplane_of(&self, other_id: u16, other_kind: MsCallKind) -> bool {
        other_kind != MsCallKind::Individual
            && self.call_identifier != other_id
            && self.kind == MsCallKind::Individual
            && self.last_uplane.is_some_and(|u| u.switch_u_plane)
    }
    pub fn locally_clearing(&self) -> bool {
        matches!(self.state, MsCcState::Disconnect | MsCcState::Release)
    }
    pub fn restoration_requests_tx(&self) -> bool {
        self.pending_tx_request || self.tx_grant_state == MsTxGrantState::GrantedSelf
    }
    pub fn needs_alert(&self) -> bool {
        self.hook_on_off
    }
    pub fn allows_dtmf(&self) -> bool {
        self.kind == MsCallKind::Individual
    }
}

pub fn resolve_floor_grant(own: Option<u32>, grant: u8, speaker: Option<u32>) -> Result<u8, CmceError> {
    if grant > 3 || own.is_some_and(|ssi| ssi > 0xff_ffff) || speaker.is_some_and(|ssi| ssi > 0xff_ffff) {
        return Err(CmceError::InvalidValue);
    }
    Ok(if grant == 0 && speaker.is_some() && speaker != own {
        3
    } else {
        grant
    })
}
pub fn ignore_group_setup(kind: MsCallKind, own: Option<u32>, caller: Option<u32>) -> bool {
    kind != MsCallKind::Individual && caller.is_some() && caller == own
}
pub fn floor_route(address: TetraAddress, own: Option<u32>, link: u32, uplane: Option<MsUPlaneState>) -> (TetraAddress, u32, bool) {
    let address = signalling_address(address, own);
    let stealing = uplane.is_some_and(|u| u.switch_u_plane);
    (address, if stealing { 2 } else { link }, stealing)
}
pub fn signalling_address(address: TetraAddress, own: Option<u32>) -> TetraAddress {
    own.map_or(address, |ssi| TetraAddress::new(ssi, SsiType::Issi))
}

#[derive(Clone, Debug)]
pub struct PendingOrigination<B> {
    pub called_party: TetraAddress,
    pub basic_service: B,
    pub simplex_duplex_selection: bool,
}

pub fn originating_service<B: Clone>(offered: Option<B>, pending: Option<&PendingOrigination<B>>, duplex: bool, fallback: B) -> (B, bool) {
    let service = offered.or_else(|| pending.map(|p| p.basic_service.clone())).unwrap_or(fallback);
    let duplex = pending.map_or(duplex, |p| p.simplex_duplex_selection);
    (service, duplex)
}

pub fn kind_from_communication(wire: u8) -> Result<MsCallKind, CmceError> {
    match wire {
        0 => Ok(MsCallKind::Individual),
        1 => Ok(MsCallKind::Group),
        2 => Ok(MsCallKind::AcknowledgedGroup),
        3 => Ok(MsCallKind::Broadcast),
        _ => Err(CmceError::InvalidValue),
    }
}
pub fn seconds_to_timeslots(seconds: i32) -> i32 {
    ((i64::from(seconds) * 12_000) / 170).clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
pub fn setup_timeout_to_timeslots(wire: u8) -> Option<i32> {
    let seconds = match wire {
        1 => 1,
        2 => 2,
        3 => 5,
        4 => 10,
        5 => 20,
        6 => 30,
        7 => 60,
        _ => return None,
    };
    Some(seconds_to_timeslots(seconds))
}
pub fn call_timeout_to_timeslots(wire: u8) -> Option<i32> {
    let seconds = match wire {
        1 => 30,
        2 => 45,
        3 => 60,
        4 => 120,
        5 => 180,
        6 => 240,
        7 => 300,
        8 => 360,
        9 => 480,
        10 => 600,
        11 => 720,
        12 => 900,
        13 => 1200,
        14 => 1800,
        _ => return None,
    };
    Some(seconds_to_timeslots(seconds))
}

/// Bounded caller-owned call table. Insertion failure never evicts a call.
pub struct CallTable<T, const N: usize> {
    slots: [Option<(u16, T)>; N],
    len: u16,
}
impl<T, const N: usize> Default for CallTable<T, N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T, const N: usize> CallTable<T, N> {
    /// # Safety
    /// `out` must be aligned, writable, exclusively owned uninitialized storage.
    /// Existing values, if any, are not dropped.
    pub unsafe fn init_at(out: *mut Self) {
        const { assert!(N <= u16::MAX as usize) };
        // SAFETY: the caller provides the complete table; initialize every slot.
        unsafe {
            let slots = core::ptr::addr_of_mut!((*out).slots).cast::<Option<(u16, T)>>();
            for index in 0..N {
                slots.add(index).write(None);
            }
            core::ptr::addr_of_mut!((*out).len).write(0);
        }
    }
    pub const fn new() -> Self {
        assert!(N <= u16::MAX as usize);
        Self {
            slots: [const { None }; N],
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn is_full(&self) -> bool {
        usize::from(self.len) == N
    }
    pub fn get(&self, id: &u16) -> Option<&T> {
        self.slots.iter().flatten().find(|(key, _)| key == id).map(|(_, value)| value)
    }
    pub fn get_mut(&mut self, id: &u16) -> Option<&mut T> {
        self.slots.iter_mut().flatten().find(|(key, _)| key == id).map(|(_, value)| value)
    }
    pub fn contains_key(&self, id: &u16) -> bool {
        self.get(id).is_some()
    }
    pub fn insert(&mut self, id: u16, value: T) -> Result<Option<T>, CmceError> {
        if id > 0x3fff {
            return Err(CmceError::InvalidValue);
        }
        if let Some(existing) = self.get_mut(&id) {
            return Ok(Some(core::mem::replace(existing, value)));
        }
        let slot = self.slots.iter_mut().find(|slot| slot.is_none()).ok_or(CmceError::Capacity)?;
        *slot = Some((id, value));
        self.len += 1;
        Ok(None)
    }
    pub fn remove(&mut self, id: &u16) -> Option<T> {
        let slot = self.slots.iter_mut().find(|slot| slot.as_ref().is_some_and(|(key, _)| key == id))?;
        self.len -= 1;
        slot.take().map(|(_, value)| value)
    }
    pub fn iter(&self) -> impl Iterator<Item = (&u16, &T)> {
        self.slots.iter().flatten().map(|(key, value)| (key, value))
    }
    pub fn values(&self) -> impl Iterator<Item = &T> {
        self.slots.iter().flatten().map(|(_, value)| value)
    }
    pub fn keys(&self) -> impl Iterator<Item = &u16> {
        self.slots.iter().flatten().map(|(key, _)| key)
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().flatten().map(|(_, value)| value)
    }
    pub fn remap(&mut self, old: u16, new: u16) -> Result<Option<T>, CmceError> {
        if new > 0x3fff {
            return Err(CmceError::InvalidValue);
        }
        if old == new {
            return Ok(None);
        }
        if !self.contains_key(&old) {
            return Err(CmceError::UnknownCall);
        }
        let replaced = self.remove(&new);
        for (key, _) in self.slots.iter_mut().flatten() {
            if *key == old {
                *key = new;
                break;
            }
        }
        Ok(replaced)
    }
}

pub struct PendingStack<T, const N: usize> {
    slots: [Option<T>; N],
    len: u16,
}
impl<T, const N: usize> Default for PendingStack<T, N> {
    fn default() -> Self {
        Self::new()
    }
}
impl<T, const N: usize> PendingStack<T, N> {
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.slots[..usize::from(self.len)].iter().flatten()
    }
    /// # Safety
    /// `out` must be aligned, writable, exclusively owned uninitialized storage.
    /// Existing values, if any, are not dropped.
    pub unsafe fn init_at(out: *mut Self) {
        const { assert!(N <= u16::MAX as usize) };
        // SAFETY: the caller provides the complete stack; initialize every slot.
        unsafe {
            let slots = core::ptr::addr_of_mut!((*out).slots).cast::<Option<T>>();
            for index in 0..N {
                slots.add(index).write(None);
            }
            core::ptr::addr_of_mut!((*out).len).write(0);
        }
    }
    pub const fn new() -> Self {
        assert!(N <= u16::MAX as usize);
        Self {
            slots: [const { None }; N],
            len: 0,
        }
    }
    pub fn len(&self) -> usize {
        usize::from(self.len)
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn is_full(&self) -> bool {
        usize::from(self.len) == N
    }
    pub fn push(&mut self, value: T) -> Result<(), CmceError> {
        if self.is_full() {
            return Err(CmceError::Capacity);
        }
        self.slots[usize::from(self.len)] = Some(value);
        self.len += 1;
        Ok(())
    }
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        self.slots[usize::from(self.len)].take()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasicService {
    pub circuit_mode: u8,
    pub encrypted: bool,
    pub communication: u8,
    pub parameter: u8,
}
impl BasicService {
    fn write(self, bits: &mut PackedBits) -> Result<(), CmceError> {
        bits.push(3, u32::from(self.circuit_mode))?;
        bits.push(1, u32::from(self.encrypted))?;
        bits.push(2, u32::from(self.communication))?;
        bits.push(2, u32::from(self.parameter))?;
        Ok(())
    }
}

/// The uplink PDU set actually generated by the MS implementation. Optional
/// Type-3 payloads are borrowed bit views, not heap-backed PDU objects.
#[derive(Clone, Copy, Debug)]
pub enum Uplink<'a> {
    Setup {
        called: u32,
        basic: BasicService,
        hook: bool,
        duplex: bool,
        request_tx: bool,
        external: Option<BitView<'a>>,
    },
    Alert {
        id: u16,
        duplex: bool,
        basic: BasicService,
    },
    Connect {
        id: u16,
        hook: bool,
        duplex: bool,
        basic: BasicService,
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
        other_party: u32,
        request_tx: bool,
        basic: BasicService,
    },
    Info {
        id: u16,
        dtmf: BitView<'a>,
    },
    TxDemand {
        id: u16,
        priority: u8,
        encrypted: bool,
    },
    TxCeased {
        id: u16,
    },
}

pub fn encode_uplink(pdu: Uplink<'_>) -> Result<PackedBits, CmceError> {
    let mut bits = PackedBits::new();
    let (kind, id) = match pdu {
        Uplink::Setup { .. } => (7, None),
        Uplink::Alert { id, .. } => (0, Some(id)),
        Uplink::Connect { id, .. } => (2, Some(id)),
        Uplink::Disconnect { id, .. } => (4, Some(id)),
        Uplink::Release { id, .. } => (6, Some(id)),
        Uplink::Restore { id, .. } => (14, Some(id)),
        Uplink::Info { id, .. } => (5, Some(id)),
        Uplink::TxDemand { id, .. } => (10, Some(id)),
        Uplink::TxCeased { id } => (9, Some(id)),
    };
    bits.push(5, kind)?;
    if let Some(id) = id {
        bits.push(14, u32::from(id))?;
    }
    match pdu {
        Uplink::Setup {
            called,
            basic,
            hook,
            duplex,
            request_tx,
            external,
        } => {
            bits.push(4, 0)?;
            bits.push(1, u32::from(hook))?;
            bits.push(1, u32::from(duplex))?;
            basic.write(&mut bits)?;
            bits.push(1, u32::from(request_tx))?;
            bits.push(4, 0)?;
            bits.push(2, 0)?;
            bits.push(2, 1)?;
            bits.push(24, called)?;
            bits.push(1, u32::from(external.is_some()))?;
            if let Some(data) = external {
                type3(&mut bits, 2, data)?;
                bits.push(1, 0)?;
            }
        }
        Uplink::Alert { duplex, basic, .. }
        | Uplink::Connect {
            hook: true, duplex, basic, ..
        } => {
            bits.push(1, 1)?;
            bits.push(1, u32::from(duplex))?;
            bits.push(2, 3)?; // o-bit and Basic service p-bit.
            basic.write(&mut bits)?;
            bits.push(1, 0)?;
        }
        Uplink::Connect {
            hook: false,
            duplex,
            basic,
            ..
        } => {
            bits.push(1, 0)?;
            bits.push(1, u32::from(duplex))?;
            bits.push(2, 3)?;
            basic.write(&mut bits)?;
            bits.push(1, 0)?;
        }
        Uplink::Disconnect { cause, .. } | Uplink::Release { cause, .. } => {
            if cause > 23 {
                return Err(CmceError::InvalidValue);
            }
            bits.push(5, u32::from(cause))?;
            bits.push(1, 0)?;
        }
        Uplink::Restore {
            other_party,
            request_tx,
            basic,
            ..
        } => {
            bits.push(1, u32::from(request_tx))?;
            bits.push(2, 1)?;
            bits.push(24, other_party)?;
            bits.push(2, 3)?;
            basic.write(&mut bits)?;
            bits.push(1, 0)?;
        }
        Uplink::Info { dtmf, .. } => {
            bits.push(1, 0)?; // poll response
            bits.push(1, 1)?; // o-bit
            bits.push(1, 0)?; // modify absent
            type3(&mut bits, 1, dtmf)?;
            bits.push(1, 0)?;
        }
        Uplink::TxDemand { priority, encrypted, .. } => {
            bits.push(2, u32::from(priority))?;
            bits.push(1, u32::from(encrypted))?;
            bits.push(2, 0)?; // reserved, o-bit
        }
        Uplink::TxCeased { .. } => bits.push(1, 0)?,
    }
    Ok(bits)
}

fn type3(bits: &mut PackedBits, id: u32, data: BitView<'_>) -> Result<(), CmceError> {
    if data.len() > 2047 {
        return Err(CmceError::InvalidValue);
    }
    bits.push(1, 1)?;
    bits.push(4, id)?;
    bits.push(11, u32::from(data.len()))?;
    bits.append(data)?;
    Ok(())
}

/// External subscriber digits, retaining the historical Unicode-whitespace
/// display convenience while bounding significant digits to clause 14.8.20.
pub fn encode_external_number(digits: &str) -> Result<PackedBits, CmceError> {
    let mut bits = PackedBits::new();
    for digit in digits.chars().filter(|digit| !digit.is_whitespace()) {
        if bits.len() == 96 {
            return Err(CmceError::InvalidValue);
        }
        let code = match digit {
            '0'..='9' => digit as u32 - u32::from(b'0'),
            '*' => 10,
            '#' => 11,
            '+' => 12,
            _ => return Err(CmceError::InvalidValue),
        };
        bits.push(4, code)?;
    }
    if bits.is_empty() {
        return Err(CmceError::InvalidValue);
    }
    Ok(bits)
}

pub fn decode_external_number(bits: BitView<'_>, output: &mut [u8; 24]) -> Result<u8, CmceError> {
    if bits.is_empty() || bits.len() > 96 || bits.len() % 4 != 0 {
        return Err(CmceError::InvalidValue);
    }
    let mut decoded = [0; 24];
    let len = (bits.len() / 4) as u8;
    for index in 0..len {
        decoded[usize::from(index)] = match bits.read(u16::from(index) * 4, 4)? {
            code @ 0..=9 => b'0' + code as u8,
            10 => b'*',
            11 => b'#',
            12 => b'+',
            _ => return Err(CmceError::InvalidValue),
        };
    }
    *output = decoded;
    Ok(len)
}

pub fn encode_dtmf(digits: Option<&[u8]>) -> Result<PackedBits, CmceError> {
    let mut bits = PackedBits::new();
    if let Some(digits) = digits {
        if digits.is_empty() || digits.len() > 254 || digits.iter().any(|digit| *digit > 15) {
            return Err(CmceError::InvalidValue);
        }
        bits.push(3, 0)?;
        for digit in digits {
            bits.push(4, u32::from(*digit))?;
        }
    } else {
        bits.push(3, 1)?;
    }
    Ok(bits)
}

#[derive(Clone, Copy, Debug)]
pub struct Dtmf<'a> {
    pub kind: u8,
    pub digits: BitView<'a>,
}

pub fn decode_dtmf(bits: BitView<'_>) -> Result<Dtmf<'_>, CmceError> {
    let kind = bits.read(0, 3)? as u8;
    let tail = bits.len() - 3;
    if kind == 0 && (tail == 0 || tail % 4 != 0 || tail / 4 > 254) {
        return Err(CmceError::InvalidValue);
    }
    Ok(Dtmf {
        kind,
        digits: bits.window(3, if kind == 0 { tail } else { 0 })?,
    })
}
